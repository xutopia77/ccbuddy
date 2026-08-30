// 业务 API 层：只面向业务，不接触 Tauri / HTTP。
// 底层统一 RPC 已在 ipc.ts 中封装，新增后端命令时在此加一个函数即可。

import { request } from "./ipc";
import type { Session } from "./types";

/** hook 安装/注册状态。 */
export interface HookStatus {
  installed: boolean;
  /** 各 hook 事件是否已在 settings.json 注册。 */
  registered: Record<string, boolean>;
  /** hook 手动下载地址（离线环境用）。 */
  downloadUrl: string;
}

/** 获取事件流会话列表（hook 日志，懒加载：messages 为空）。 */
export const getEvents = () => request<Session[]>("get_events");

/** 获取会话列表（Claude Code 原生 transcript，与事件流分开）。 */
export const getSessions = () => request<Session[]>("get_sessions");

/** 按需加载单个会话的完整消息（用户点开会话详情时调用）。 */
export const getSessionDetail = (id: string) => request<Session>("get_session_detail", id);

/** 用户配置（get_config 返回的视图，含只读派生字段）。 */
export interface AppConfig {
  claude_dir: string;
  github_repo: string;
  /** 只读：日志源目录 */
  events_dir: string;
  /** 只读：ccbuddy 数据根目录 */
  data_root: string;
  /** 当前日志等级（修改走 setConfig 的 log_level） */
  log_level: string;
}

/** 读取用户配置。 */
export const getConfig = () => request<AppConfig>("get_config");

/**
 * 部分更新用户配置：只传要改的字段。
 * 可写：claude_dir / github_repo / log_level；未知或只读字段会报错。
 */
export const setConfig = (patch: Partial<Pick<AppConfig, "claude_dir" | "github_repo" | "log_level">>) =>
  request<AppConfig>("set_config", patch);

/** 一键安装 hook（复制 ccbuddy-hook 并注册到 settings.json）。 */
export const installHooks = () => request<string>("install_hooks");

/** 获取 hook 安装/注册状态（installed + 各事件 registered）。 */
export const getHookStatus = () => request<HookStatus>("get_hook_status");
