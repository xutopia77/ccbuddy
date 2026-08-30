// 业务 API 层：只面向业务，不接触 Tauri / HTTP。
// 底层统一 RPC 已在 ipc.ts 中封装，新增后端命令时在此加一个函数即可。

import { request } from "./ipc";
import type { Session } from "./types";

/** hook 安装/注册状态。 */
export interface HookStatus {
  installed: boolean;
  /** 各 hook 事件是否已在 settings.json 注册。 */
  registered: Record<string, boolean>;
}

/** 获取会话列表（含 hook 日志实时会话 + 原生历史会话，懒加载：messages 为空）。 */
export const getSessions = () => request<Session[]>("get_sessions");

/** 按需加载单个会话的完整消息（用户点开会话详情时调用）。 */
export const getSessionDetail = (id: string) => request<Session>("get_session_detail", id);

/** 获取日志源目录路径。 */
export const getEventsDir = () => request<string>("get_events_dir");

/** 一键安装 hook（复制 ccbuddy-hook 并注册到 settings.json）。 */
export const installHooks = () => request<string>("install_hooks");

/** 获取 hook 安装/注册状态（installed + 各事件 registered）。 */
export const getHookStatus = () => request<HookStatus>("get_hook_status");

/** 运行时设置日志打印等级（error/warn/info/debug/trace）。 */
export const setLogLevel = (level: string) => request<string>("set_log_level", level);
