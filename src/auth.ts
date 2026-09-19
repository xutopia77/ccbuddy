// 登录态：仅浏览器访问 ccbuddy-server 时有意义（桌面端 Tauri 无登录概念）。
//
// 服务端会话机制见 `src-tauri/src/auth.rs`：登录成功签发 token（HttpOnly cookie），
// 空闲 15 分钟 / 最长 15 天失效，服务重启即全部失效（只存内存）。
// 密码本身由服务端校验，本模块只负责「当前是否已登录」与那几个请求。

import { ref } from "vue";
import { isTauri, onUnauthorized, request } from "./ipc";
import { t } from "./i18n";

/**
 * 登录态。**三态**：
 * - `null` = 尚未探测（首屏什么都不渲染，避免闪一下登录页）
 * - `true` / `false` = 已确定
 *
 * 桌面端短路为 `true`：Tauri 用进程内 invoke 通信，压根没有 /api/session 这个服务，
 * 若也去探测，桌面版会永远卡在登录页。
 */
export const authed = ref<boolean | null>(isTauri ? true : null);

/** 心跳周期：60 秒。服务端空闲过期是 15 分钟，粒度足够且开销可忽略。 */
const HEARTBEAT_MS = 60_000;

/**
 * 探测登录态并同步到 [`authed`]。
 *
 * `/api/session` 返回 200 + `{authed}`，所以这里只在网络/服务异常时按未登录处理。
 * 它同时是发现「服务重启导致 token 全部失效」的唯一途径——SSE 断线不给状态码。
 */
export async function refreshSession(): Promise<void> {
  try {
    // fetch 默认 credentials: "same-origin"，同源请求会自动带上 session cookie
    const r = await fetch("/api/session");
    if (!r.ok) {
      authed.value = false;
      return;
    }
    const body = (await r.json()) as { authed?: boolean };
    authed.value = body.authed === true;
  } catch {
    // 服务没起来 / 网络断开：按未登录处理（登录页会提示连不上）
    authed.value = false;
  }
}

/** 登录。密码错误抛错（消息可直接展示给用户）。 */
export async function login(password: string): Promise<void> {
  let r: Response;
  try {
    r = await fetch("/api/login", {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ password }),
    });
  } catch {
    throw new Error(t("loginNetFail"));
  }
  if (!r.ok) throw new Error(t("loginFailed"));
  authed.value = true;
}

/** 退出登录：服务端撤销当前 token 并清 cookie，本地态归零。 */
export async function logout(): Promise<void> {
  try {
    await fetch("/api/logout", { method: "POST" });
  } catch {
    // 网络异常也照样清本地态：cookie 由服务端撤销，页面先回登录页
  }
  authed.value = false;
}

/**
 * 修改访问密码。服务端改完会撤销**全部**会话（含当前标签页），
 * 所以这里跟着把登录态归零，界面回登录页。
 */
export async function changePassword(oldPwd: string, newPwd: string): Promise<void> {
  await request("change_password", { old: oldPwd, new: newPwd });
  authed.value = false;
}

/** 是否已启动（防重复注册心跳）。 */
let started = false;

/**
 * 启动登录态维护：首屏探测 + 全局 401 回调 + 可见时心跳。桌面端直接跳过。
 *
 * 「活跃」的判定在服务端，前端只负责产生信号：**标签页可见时**才发心跳。
 * 切到后台不发——人不在看就不算活跃，15 分钟后自动登出，这正是想要的行为。
 */
export function startAuth(): void {
  if (started || isTauri) return;
  started = true;

  // 任意 RPC 拿到 401（会话过期 / 服务重启 / 改密码）立即回登录页，
  // 不必等下一次心跳——心跳最长 60 秒才打一次。
  // 订阅（含 SSE 连接）的建立与关闭由 App.vue 的 watch(authed) 统一管理。
  onUnauthorized(() => {
    authed.value = false;
  });

  setInterval(() => {
    if (document.visibilityState !== "visible") return;
    void refreshSession();
  }, HEARTBEAT_MS);

  // 从后台切回前台立即补一次：可能已经过期，早点回登录页而不是等下一次心跳
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") void refreshSession();
  });

  void refreshSession();
}
