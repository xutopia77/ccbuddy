export type SessionStatus =
  | "running"
  | "waiting_confirmation"
  | "waiting_input"
  | "error"
  | "completed"
  | "idle";

export interface Message {
  type: "user" | "assistant" | "thinking" | "tool_use" | "tool_result" | "system";
  role: "user" | "assistant" | "system";
  content: string;
  time: string;
  toolCall?: string;
}

/** 子代理转录概要（历史会话详情携带，M-C3）。 */
export interface SubagentInfo {
  /** 子代理名（文件名派生，如 agent-a1444bf34ebff26b2） */
  id: string;
  /** 任务描述（meta.json 的 description，缺失为空串） */
  description: string;
}

export interface Session {
  id: string;
  project: string;
  cwd: string;
  title: string;
  status: SessionStatus;
  lastActivity: string;
  preview: string;
  unread: boolean;
  messages: Message[];
  /** 原生 transcript 路径（事件流会话携带，历史会话解析不提供） */
  transcriptPath?: string;
  /** SessionStart 来源（startup / resume / clear / compact / fork） */
  source?: string;
  /** 子代理转录列表（历史会话详情携带，无子代理时不出现） */
  subagents?: SubagentInfo[];
}

export type View = "sessions" | "history-sessions" | "usage" | "settings";

/** 单次 API 请求的 token 用量记录（后端 usage_mgr 解析原生 transcript）。 */
export interface UsageRecord {
  /** 请求时间（本地时区 ISO） */
  timestamp: string;
  sessionId: string;
  /** 模型名（缺失为空串） */
  model: string;
  inputTokens: number;
  cacheReadTokens: number;
  cacheCreationTokens: number;
  outputTokens: number;
  /** 思考 token（部分模型/版本才有） */
  thinkingTokens?: number;
  /** 服务档位（部分版本才有） */
  serviceTier?: string;
  /** 本次请求成本（美元，新版 transcript 才有成本字段） */
  cost?: number;
  /** 是否子代理（subagent）转录产生的用量 */
  isSubagent?: boolean;
}

export function statusColor(status: SessionStatus): string {
  const map: Record<SessionStatus, string> = {
    running: "var(--success)",
    waiting_confirmation: "var(--warning)",
    waiting_input: "var(--info)",
    error: "var(--danger)",
    completed: "var(--muted)",
    idle: "var(--text-3)",
  };
  return map[status] ?? "var(--muted)";
}

import { t, locale, type I18nKey } from "./i18n";

/** 状态六态修饰类名（EventList 状态标签 / 详情大徽章共用配色类） */
export function dotCls(status: SessionStatus): string {
  const map: Record<SessionStatus, string> = {
    running: "running",
    waiting_confirmation: "confirm",
    waiting_input: "input",
    error: "error",
    completed: "done",
    idle: "idle",
  };
  return map[status] ?? "idle";
}

/** 状态徽章文案键（六态；语言切换时随 locale 重算） */
const STATUS_KEY: Record<SessionStatus, I18nKey> = {
  running: "statusRunning",
  waiting_confirmation: "statusConfirm",
  waiting_input: "statusInput",
  error: "statusError",
  completed: "statusDone",
  idle: "statusIdle",
};

export function statusLabel(status: SessionStatus): string {
  return t(STATUS_KEY[status] ?? "statusIdle");
}

/// ISO 时间戳 → 相对时间（zh：刚刚/N分钟前…；en：just now/5m ago…）。
/// 读取全局 locale，语言切换时各计算属性自然重算。
export function fmtRelative(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const sec = Math.floor((Date.now() - d.getTime()) / 1000);
  if (sec < 60) return t("relNow");
  const min = Math.floor(sec / 60);
  if (min < 60) return locale.value === "zh" ? `${min}分钟前` : `${min}m ago`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return locale.value === "zh" ? `${hr}小时前` : `${hr}h ago`;
  const day = Math.floor(hr / 24);
  if (day < 7) return locale.value === "zh" ? `${day}天前` : `${day}d ago`;
  return d.toLocaleDateString(locale.value === "zh" ? "zh-CN" : "en-US");
}

/// ISO 时间戳 → "YYYY-MM-DD HH:mm:ss"（本地时区）。
export function fmtDateTime(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}