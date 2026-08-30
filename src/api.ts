// 业务 API 层：只面向业务，不接触 Tauri / HTTP。
// 底层统一 RPC 已在 ipc.ts 中封装，新增后端命令时在此加一个函数即可。

import { request } from "./ipc";
import type { Session } from "./types";

/** 获取会话列表（含 hook 日志实时会话 + 原生历史会话）。 */
export const getSessions = () => request<Session[]>("get_sessions");

/** 获取日志源目录路径。 */
export const getEventsDir = () => request<string>("get_events_dir");

/** 一键安装 hook（复制 ccbuddy-hook 并注册到 settings.json）。 */
export const installHooks = () => request<string>("install_hooks");
