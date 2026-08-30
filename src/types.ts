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
}

export type View = "sessions" | "history-sessions" | "settings";

/** 会话列表轮询配置（纯前端量，localStorage 持久化）。 */
export interface PollConfig {
  enabled: boolean;
  intervalSec: number;
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

export function statusLabel(status: SessionStatus): string {
  const map: Record<SessionStatus, string> = {
    running: "运行中",
    waiting_confirmation: "需确认",
    waiting_input: "等待输入",
    error: "异常",
    completed: "已完成",
    idle: "空闲",
  };
  return map[status] ?? status;
}

/// ISO 时间戳 → 相对时间（"刚刚"/"N分钟前"/"N小时前"/"N天前"）。
export function fmtRelative(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const sec = Math.floor((Date.now() - d.getTime()) / 1000);
  if (sec < 60) return "刚刚";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}分钟前`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}小时前`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}天前`;
  return d.toLocaleDateString("zh-CN");
}

/// ISO 时间戳 → "YYYY-MM-DD HH:mm:ss"（本地时区）。
export function fmtDateTime(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`;
}