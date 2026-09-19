/**
 * 「需关注」事件的已读确认（纯前端状态，不改后端协议）。
 *
 * 背景：后端 SessionInfo.unread 是状态派生量——只要会话处于
 * 等待确认 / 等待输入 / 异常，它恒为 true，后端并不知道用户看没看过。
 * 于是左侧列表会一直高亮提醒，用户永远甩不掉。
 *
 * 方案：按「会话 id → 该状态的 lastActivity」记一条已读。
 * 用户点开会话即视为确认，此后同一状态不再高亮；若又来新事件
 * （lastActivity 变化）则重新提醒——这正是「处理完了就不再吵，
 * 有新动静再吵」的语义。
 *
 * 持久化到 localStorage：刷新/重启后不重复提醒。条目数有上限，
 * 超出后丢弃最早的一半，不会无限增长。
 */
import { ref } from "vue";
import type { Session } from "./types";

const STORAGE_KEY = "ccbuddy-ack";
const MAX_ENTRIES = 200;

function load(): Record<string, string> {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    const parsed: unknown = raw ? JSON.parse(raw) : null;
    if (parsed && typeof parsed === "object" && !Array.isArray(parsed)) {
      return parsed as Record<string, string>;
    }
  } catch {
    /* localStorage 不可用或内容损坏时按空处理 */
  }
  return {};
}

/** 已确认记录：会话 id → 已确认到的 lastActivity */
const acked = ref<Record<string, string>>(load());

/** 该会话当前状态是否已被用户确认过（lastActivity 未变即视为已读）。 */
export function isAcked(session: Session): boolean {
  return acked.value[session.id] === session.lastActivity;
}

/** 确认已读：记下该会话当前的 lastActivity。 */
export function ack(session: Session): void {
  const next: Record<string, string> = { ...acked.value, [session.id]: session.lastActivity };
  const keys = Object.keys(next);
  if (keys.length > MAX_ENTRIES) {
    for (const k of keys.slice(0, keys.length - MAX_ENTRIES / 2)) delete next[k];
  }
  acked.value = next;
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(next));
  } catch {
    /* 持久化失败不影响当次会话 */
  }
}
