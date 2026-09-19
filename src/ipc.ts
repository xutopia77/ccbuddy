// 统一 RPC 客户端：屏蔽 Tauri（invoke）与浏览器（HTTP）的差异。
//
// 请求与响应格式见后端 `src-tauri/src/rpc.rs`：
//   请求 { time, cmd, data }
//   响应 { time, cmd, code, status, data }
// 业务层只调用 request<T>(cmd, data)，不感知底层实现。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type { UnlistenFn };

/** 请求体（前端 → 后端）。 */
export interface RpcRequest {
  time: string;
  cmd: string;
  data?: unknown;
}

/** 响应体（后端 → 前端，事件推送复用同一结构）。 */
export interface RpcResponse<T = unknown> {
  time: string;
  cmd: string;
  code: number;
  status: string;
  data: T;
}

/** 成功状态码（与后端 rpc.rs 保持一致）。 */
export const CODE_OK = 0;

/** 业务错误：请求失败（code !== 0）或网络异常时抛出。 */
export class RpcError extends Error {
  readonly code: number;
  constructor(code: number, status: string) {
    super(status);
    this.name = "RpcError";
    this.code = code;
  }
}

/** 是否运行在 Tauri WebView 中（否则为浏览器 / ccbuddy-server）。 */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** 当前 UTC 毫秒时间戳，如 `2026-08-30T12:34:56.789Z`。 */
function nowMs(): string {
  return new Date().toISOString();
}

/**
 * 调用后端命令，返回命令的数据部分。
 * 非 0 状态码会抛出 RpcError。
 */
export async function request<T = unknown>(cmd: string, data?: unknown): Promise<T> {
  const payload: RpcRequest = { time: nowMs(), cmd, data: data ?? null };

  let res: RpcResponse<T>;
  if (isTauri) {
    res = await invoke<RpcResponse<T>>("rpc", { payload });
  } else {
    // fetch 默认 credentials: "same-origin"，同源请求自动带上 session cookie
    const r = await fetch("/api/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (r.status === HTTP_UNAUTHORIZED) {
      // 会话过期 / 服务重启 / 改密码：通知登录态归零，界面回登录页
      notifyUnauthorized();
      throw new RpcError(r.status, `HTTP ${r.status}`);
    }
    if (!r.ok) throw new RpcError(r.status, `HTTP ${r.status}`);
    res = (await r.json()) as RpcResponse<T>;
  }

  if (res.code !== CODE_OK) {
    throw new RpcError(res.code, res.status);
  }
  return res.data;
}

/** 服务端判定未登录的 HTTP 状态码（与 server.rs 的 unauthorized 一致）。 */
const HTTP_UNAUTHORIZED = 401;

// ---- 全局「未登录」信号 ----
// 用回调而不是让 ipc.ts 直接 import auth.ts：auth.ts 依赖 ipc.ts 发请求，
// 反向 import 会成环。
const unauthCbs = new Set<() => void>();

/** 订阅「请求被服务端判定未登录」。返回取消订阅函数。 */
export function onUnauthorized(cb: () => void): () => void {
  unauthCbs.add(cb);
  return () => {
    unauthCbs.delete(cb);
  };
}

function notifyUnauthorized() {
  for (const cb of [...unauthCbs]) cb();
}

/**
 * 订阅后端主动推送的事件。
 * 后端推送的即 RpcResponse 结构，handler 收到其中的 `data`。
 * 返回取消订阅函数。
 *
 * - Tauri：listen 监听 emit 的事件
 * - 浏览器：共享一条 SSE 连接（/api/events），按 cmd 分发给订阅者
 */
export function onEvent<T = unknown>(
  event: string,
  handler: (data: T) => void
): UnlistenFn {
  if (isTauri) {
    let unlisten: UnlistenFn | null = null;
    void listen<RpcResponse<T>>(event, (e) => handler(e.payload.data))
      .then((fn) => (unlisten = fn))
      .catch(console.error);
    return () => unlisten?.();
  }
  return sseSubscribe(event, handler as (data: unknown) => void) as UnlistenFn;
}

// ---- 浏览器 SSE 共享连接 ----

/** SSE 事件订阅者表：cmd → 回调集合。 */
const sseHandlers = new Map<string, Set<(data: unknown) => void>>();

/** 共享 EventSource（惰性建立，首个订阅者出现时连接）。 */
let sseSource: EventSource | null = null;

// ---- SSE 连接状态信号（顶栏存活指示用：连接状态比"最近收到业务推送"可靠，
// 系统安静时后端不推业务事件，但连接仍然健康） ----
const sseConnCbs = new Set<(ok: boolean) => void>();

function notifySseConn(ok: boolean) {
  for (const cb of [...sseConnCbs]) cb(ok);
}

/** 订阅 SSE 连接状态变化（open → ok=true，error/断线 → ok=false）。
 * 注册时立即按当前连接状态回调一次。返回取消订阅函数。 */
export function onSseConnectionChange(cb: (ok: boolean) => void): () => void {
  sseConnCbs.add(cb);
  // source 已存在按 readyState 回调；尚未建连（注册早于首个订阅者）先回调 false，
  // 建连后 onopen 会再次回调
  cb(sseSource?.readyState === EventSource.OPEN);
  return () => {
    sseConnCbs.delete(cb);
  };
}

/**
 * 关闭共享 SSE 连接（登出时调用；重新登录后由首个订阅者重建）。
 *
 * 必须真的 close 并置空：未登录时建连会拿到 401 进入 CLOSED 且不重试，
 * 若只摘监听不关连接，登录后所有订阅都挂在这条死连接上——页面能显示、
 * 能拉到一次数据，但再也收不到任何推送。
 */
export function closeSse() {
  sseSource?.close();
  sseSource = null;
  sseHandlers.clear();
  notifySseConn(false);
}

/** 订阅 SSE 事件：共享一条连接，按 cmd 分发。返回取消订阅函数。 */
function sseSubscribe(event: string, handler: (data: unknown) => void): () => void {
  let set = sseHandlers.get(event);
  if (!set) {
    set = new Set();
    sseHandlers.set(event, set);
    sseSource?.addEventListener(event, sseDispatch);
  }
  set.add(handler);

  // 惰性建连：首个订阅者出现时打开 EventSource
  if (!sseSource) {
    const src = new EventSource("/api/events");
    sseSource = src;
    // 连接状态：onopen 即通，onerror（含自动重连期间）即断；重连成功会再触发 onopen
    src.onopen = () => notifySseConn(true);
    src.onerror = () => {
      notifySseConn(false);
      // 未登录时建连会拿到 401，按 SSE 规范这属于「连接失败」：直接 CLOSED 且**不重试**。
      // 置空让下一个订阅者重建连接，登录后才能恢复推送。
      // 只在 CLOSED 时置空——CONNECTING 是浏览器自己的重连窗口，置空会开出第二条流。
      if (src.readyState === EventSource.CLOSED && sseSource === src) sseSource = null;
    };
    // 连接建立后，对已注册的 cmd 重新挂监听（addEventListener 在 open 前也有效，
    // EventSource 构造即开始连接，此处无时序问题）
    for (const cmd of sseHandlers.keys()) {
      src.addEventListener(cmd, sseDispatch);
    }
  }
  return () => {
    const s = sseHandlers.get(event);
    s?.delete(handler);
    if (s && s.size === 0) {
      sseHandlers.delete(event);
      sseSource?.removeEventListener(event, sseDispatch);
    }
  };
}

/** SSE 帧分发：解析 RpcResponse 信封，投递 data 给对应 cmd 的订阅者。 */
function sseDispatch(ev: MessageEvent<string>) {
  let resp: RpcResponse | undefined;
  try {
    resp = JSON.parse(ev.data) as RpcResponse;
  } catch {
    console.error("[ipc] SSE 帧解析失败", ev.data);
    return;
  }
  const handlers = sseHandlers.get(resp.cmd);
  if (!handlers) return;
  for (const h of [...handlers]) {
    try {
      h(resp.data);
    } catch (e) {
      console.error("[ipc] SSE handler 异常", e);
    }
  }
}
