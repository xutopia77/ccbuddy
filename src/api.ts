// 业务 API 层：只面向业务，不接触 Tauri / HTTP。
// 底层统一 RPC 已在 ipc.ts 中封装，新增后端命令时在此加一个函数即可。

import { request } from "./ipc";
import type { Session, UsageRecord } from "./types";

/** hook 安装/注册状态。 */
export interface HookStatus {
  installed: boolean;
  /** 各 hook 事件是否已在 settings.json 注册。 */
  registered: Record<string, boolean>;
  /** 状态异常（已注册但 hook 文件缺失 / Unix 上无可执行位）。 */
  broken: boolean;
  /** broken 时的具体原因与修复提示。 */
  broken_reason: string | null;
}

/** 获取事件流会话列表（hook 日志，懒加载：messages 为空）。 */
export const getEvents = () => request<Session[]>("get_events");

/** 获取会话列表（Claude Code 原生 transcript，与事件流分开）。 */
export const getSessions = () => request<Session[]>("get_sessions");

/** 事件流会话详情（hook 日志，最新 50 条事件）。 */
export const getEventDetail = (id: string) => request<Session>("get_event_detail", id);

/** 历史会话详情（原生 transcript，全量消息）。 */
export const getSessionDetail = (id: string) => request<Session>("get_session_detail", id);

/** 全部 API 请求的 token 用量记录（时间倒序，前端自行分页/筛选/汇总）。 */
export const getUsage = () => request<UsageRecord[]>("get_usage");

/** Claude Code 官方 transcript 保留期天数（cleanupPeriodDays，只读；未配置返回 null）。 */
export const getTranscriptRetention = () => request<number | null>("get_transcript_retention");

/** 用户配置（get_config 返回的视图，含只读派生字段）。 */
export interface AppConfig {
  claude_dir: string;
  /** 用量自动刷新周期（秒，后端钳制 [5, 3600]，默认 30） */
  usage_refresh_secs: number;
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
 * 可写：claude_dir / log_level；未知或只读字段会报错。
 */
export const setConfig = (patch: Partial<Pick<AppConfig, "claude_dir" | "usage_refresh_secs" | "log_level">>) =>
  request<AppConfig>("set_config", patch);

/** 一键安装 hook（复制 ccbuddy-hook 并注册到 settings.json）。 */
export const installHooks = () => request<string>("install_hooks");

/** 卸载 hook（移除 ccbuddy-hook 注册并删除已安装的 hook 文件，其他配置不受影响）。 */
export const uninstallHooks = () => request<string>("uninstall_hooks");

/** 获取 hook 安装/注册状态（installed + 各事件 registered + 一致性 broken）。 */
export const getHookStatus = () => request<HookStatus>("get_hook_status");
