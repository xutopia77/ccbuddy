// 统一 RPC 客户端：屏蔽 Tauri（invoke）与浏览器（HTTP）的差异。
//
// 请求与响应格式见后端 `src-tauri/src/rpc.rs`：
//   请求 { time, cmd, data }
//   响应 { time, cmd, code, status, data }
// 业务层只调用 request<T>(cmd, data)，不感知底层实现。

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

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
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

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
    const r = await fetch("/api/rpc", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(payload),
    });
    if (!r.ok) throw new RpcError(r.status, `HTTP ${r.status}`);
    res = (await r.json()) as RpcResponse<T>;
  }

  if (res.code !== CODE_OK) {
    throw new RpcError(res.code, res.status);
  }
  return res.data;
}

/**
 * 订阅后端主动推送的事件。
 * 后端推送的即 RpcResponse 结构，handler 收到其中的 `data`。
 * 返回取消订阅函数。
 */
export async function onEvent<T = unknown>(
  event: string,
  handler: (data: T) => void
): Promise<UnlistenFn> {
  if (isTauri) {
    return listen<RpcResponse<T>>(event, (e) => handler(e.payload.data));
  }
  // 浏览器环境：预留 SSE 实现（EventSource 连接 /api/events）。
  console.warn(`[ipc] 浏览器环境尚未实现事件推送: ${event}`);
  return () => {};
}
