<script setup lang="ts">
import { ref, watch, computed } from "vue";
import type { Message, Session } from "../types";
import { statusColor, statusLabel, fmtDateTime } from "../types";

const props = defineProps<{
  session: Session | null;
  emptyText?: string;
  /** events：事件流时间线（实时视图）；chat：聊天记录（历史会话） */
  mode?: "events" | "chat";
}>();

/** 恢复会话的完整命令 */
function resumeCommand(s: Session): string {
  return `claude --resume ${s.id}`;
}

const copied = ref(false);

/** 复制整条恢复命令 */
function copyResume(s: Session) {
  navigator.clipboard?.writeText(resumeCommand(s)).then(() => {
    copied.value = true;
    setTimeout(() => (copied.value = false), 1500);
  });
}

interface BadgeInfo {
  icon: string;
  label: string;
  cls: string;
  action: string;
}

function badgeOf(msg: Message): BadgeInfo {
  switch (msg.type) {
    case "user":
      return { icon: "📝", label: "USER", cls: "badge-user", action: "用户输入" };
    case "assistant":
      return { icon: "🤖", label: "MSG", cls: "badge-msg", action: "Claude" };
    case "thinking":
      return { icon: "💭", label: "THINK", cls: "badge-think", action: "思考" };
    case "tool_use":
      return { icon: "🔧", label: "TOOL", cls: "badge-tool", action: "调用工具" };
    case "tool_result":
      return { icon: "✅", label: "TOOL", cls: "badge-tool-done", action: "工具完成" };
    default:
      return { icon: "⚠️", label: "SYS", cls: "badge-sys", action: "系统消息" };
  }
}

/** tool_use 内容首行为"调用工具 X"，拆出剩余入参 */
function toolBody(msg: Message): string {
  const idx = msg.content.indexOf("\n");
  return idx === -1 ? "" : msg.content.slice(idx + 1);
}

const SUMMARY_KEYS = [
  "file_path", "command", "url", "pattern", "path", "description", "query", "content",
];

/** 生成一行概要（不展示详细内容）；n 为截断长度 */
function summarize(msg: Message, n = 90): string {
  const clip = (s: string, limit = n) => {
    const t = s.replace(/\s+/g, " ").trim();
    return t.length > limit ? t.slice(0, limit) + "..." : t;
  };

  if (msg.type === "tool_use") {
    const body = toolBody(msg);
    if (!body) return msg.toolCall || "";
    // 尝试从 JSON 入参中提取最能描述意图的字段
    try {
      const input = JSON.parse(body) as Record<string, unknown>;
      for (const k of SUMMARY_KEYS) {
        if (typeof input[k] === "string" && input[k]) return clip(String(input[k]));
      }
    } catch {
      // 非 JSON 入参，直接取首行
    }
    return clip(body.split("\n")[0] || "");
  }
  return clip(msg.content);
}

/** 是否有可展开的详情 */
function hasDetail(msg: Message): boolean {
  if (msg.type === "thinking") return true;
  if (msg.type === "tool_result") return !!msg.content.trim();
  if (msg.type === "tool_use") return !!toolBody(msg).trim();
  return msg.content.length > 90;
}

/** 详情全文 */
function detailOf(msg: Message): string {
  return msg.type === "tool_use" ? toolBody(msg) : msg.content;
}

// 展开状态（按消息下标，切换会话时重置）
const expanded = ref(new Set<number>());
watch(() => props.session?.id, () => {
  expanded.value = new Set();
});

function toggle(idx: number, msg: Message) {
  if (!hasDetail(msg)) return;
  const set = new Set(expanded.value);
  set.has(idx) ? set.delete(idx) : set.add(idx);
  expanded.value = set;
}

/** 聊天模式下是否折叠显示（工具调用/输出/思考过程折叠，其余直接展示） */
function isCollapsedInChat(msg: Message): boolean {
  return msg.type === "thinking" || msg.type === "tool_use" || msg.type === "tool_result";
}

// ---- 历史会话：用户输入快速定位列表 ----
/** 仅用户输入（过滤掉系统标记类消息） */
const userInputs = computed(() => {
  if (!props.session) return [];
  return props.session.messages
    .map((msg, idx) => ({ msg, idx }))
    .filter(({ msg }) => msg.type === "user");
});

const chatBodyEl = ref<HTMLElement | null>(null);
const activeUserIdx = ref<number | null>(null);

/** 点击某条用户输入：滚动定位到聊天流中对应的消息 */
function scrollToUser(idx: number) {
  activeUserIdx.value = idx;
  const body = chatBodyEl.value;
  if (!body) return;
  const target = body.querySelector(`[data-msg-idx="${idx}"]`) as HTMLElement | null;
  if (target) {
    target.scrollIntoView({ behavior: "smooth", block: "center" });
    // 闪烁提示定位位置
    target.classList.add("flash-target");
    setTimeout(() => target.classList.remove("flash-target"), 1200);
  }
}
</script>

<template>
  <div class="detail-panel" :class="mode === 'chat' ? 'chat-mode' : 'tui'">
    <template v-if="session">
      <!-- 会话信息头 -->
      <div class="session-header">
        <!-- 第一行：会话名称 + 项目全路径 -->
        <div class="header-row">
          <span class="session-title">{{ session.title }}</span>
          <span class="session-path">{{ session.cwd || session.project }}</span>
        </div>
        <!-- 第二行：最后活动时间 + 恢复命令（文本可选中复制 id，按钮复制整条命令） -->
        <div class="header-row">
          <span class="header-time">{{ fmtDateTime(session.lastActivity) }}</span>
          <code class="resume-cmd">{{ resumeCommand(session) }}</code>
          <button class="copy-btn" @click="copyResume(session)">{{ copied ? "已复制" : "复制" }}</button>
        </div>
        <!-- 第三行：最新状态（仅事件流视图显示） -->
        <div v-if="mode !== 'chat'" class="header-row">
          <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
          <span class="status-text">{{ statusLabel(session.status) }}</span>
        </div>
      </div>

      <!-- ===== 聊天记录模式（历史会话）===== -->
      <template v-if="mode === 'chat'">
        <div class="chat-content-row">
          <div ref="chatBodyEl" class="chat-body">
            <template v-for="(msg, idx) in session.messages" :key="idx">
              <!-- 折叠块：工具调用 / 工具输出 / 思考过程 -->
              <div v-if="isCollapsedInChat(msg)" class="chat-tool" @click="toggle(idx, msg)">
                <span class="chat-tool-icon">{{ badgeOf(msg).icon }}</span>
                <span class="chat-tool-name">{{ msg.toolCall || badgeOf(msg).action }}</span>
                <span class="chat-tool-desc">{{ summarize(msg) }}</span>
                <span class="expand-arrow">{{ expanded.has(idx) ? "▾" : "▸" }}</span>
                <pre v-if="expanded.has(idx)" class="event-detail">{{ detailOf(msg) }}</pre>
              </div>
              <!-- 直接展示：用户 / 助手 / 系统 -->
              <div v-else class="chat-msg" :class="msg.type" :data-msg-idx="idx">
                <div class="chat-bubble">{{ msg.content }}</div>
              </div>
            </template>
            <div class="bottom-spacer"></div>
          </div>
          <!-- 右侧：用户输入快速定位列表 -->
          <aside v-if="userInputs.length > 0" class="user-inputs-panel">
            <div class="user-inputs-header">用户输入 {{ userInputs.length }}</div>
            <div
              v-for="({ msg, idx }, i) in userInputs"
              :key="idx"
              class="user-input-item"
              :class="{ active: activeUserIdx === idx }"
              @click="scrollToUser(idx)"
            >
              <span class="user-input-idx">{{ i + 1 }}</span>
              <span class="user-input-text">{{ summarize(msg, 40) }}</span>
            </div>
          </aside>
        </div>
      </template>

      <!-- ===== 事件流时间线模式（实时视图） ===== -->
      <div v-else class="event-stream">
        <div class="stream-header">事件流时间线 · 共 {{ session.messages.length }} 条 · 点击行展开详情</div>
        <ul class="event-list">
          <li
            v-for="(msg, idx) in session.messages"
            :key="idx"
            class="event-item"
            :class="{ expandable: hasDetail(msg) }"
            @click="toggle(idx, msg)"
          >
            <span class="event-time">{{ msg.time }}</span>
            <span class="event-badge" :class="badgeOf(msg).cls">
              <i class="icon">{{ badgeOf(msg).icon }}</i>{{ badgeOf(msg).label }}
            </span>
            <div class="event-content">
              <div class="event-line">
                <span class="event-action">
                  {{ badgeOf(msg).action }}<template v-if="msg.toolCall"> {{ msg.toolCall }}</template>
                </span>
                <span class="event-desc">{{ summarize(msg) }}</span>
                <span v-if="hasDetail(msg)" class="expand-arrow">{{ expanded.has(idx) ? "▾" : "▸" }}</span>
              </div>
              <pre v-if="expanded.has(idx)" class="event-detail">{{ detailOf(msg) }}</pre>
            </div>
          </li>
        </ul>
        <div class="bottom-spacer"></div>
      </div>
    </template>
    <n-empty
      v-else
      :description="emptyText || '选择一个会话查看详情'"
      style="margin: auto"
    />
  </div>
</template>

<style scoped>
/* ===== 公共：面板基调（design token，见 styles/tokens.css） ===== */
.detail-panel {
  position: relative; /* chat 模式的 session-header 绝对定位锚点 */
  background: var(--bg-base);
  font-family: var(--font-mono);
  font-size: 13px;
  line-height: 1.6;
}

/* 会话信息头 */
.session-header {
  padding: 10px 20px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 4px;
}
.header-row {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.session-title {
  color: var(--text-1);
  font-weight: bold;
  font-size: 14px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex-shrink: 1;
  min-width: 0;
}
.session-path {
  color: var(--text-2);
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.header-time {
  color: var(--text-2);
  font-size: 12px;
  flex-shrink: 0;
}
.resume-cmd {
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--accent-mute);
  background: var(--bg-code);
  padding: 2px 8px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
  user-select: text;
}
.copy-btn {
  background: var(--bg-code);
  border: 1px solid var(--border);
  color: var(--text-2);
  font-family: var(--font-mono);
  font-size: 12px;
  padding: 2px 10px;
  border-radius: var(--radius-md);
  cursor: pointer;
  flex-shrink: 0;
  transition: all 0.15s;
}
.copy-btn:hover { background: var(--bg-hover); border-color: var(--accent); color: var(--accent-mute); }
.status-dot {
  width: 10px;
  height: 10px;
  border-radius: 50%;
  flex-shrink: 0;
}
.status-text {
  color: var(--text-2);
  font-size: 12px;
}

/* 展开箭头与详情块（两种模式共用） */
.expand-arrow {
  color: var(--text-3);
  font-size: 11px;
  flex-shrink: 0;
}
.event-detail {
  margin: 6px 0 2px;
  padding: 8px 10px;
  font-family: var(--font-mono);
  font-size: 12px;
  color: var(--accent-mute);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-left: 2px solid var(--accent);
  border-radius: var(--radius-sm);
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 320px;
  overflow-y: auto;
  cursor: default;
}
.bottom-spacer { height: 40px; flex-shrink: 0; }

/* ===== 事件流时间线模式 ===== */
.event-stream {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
}
.stream-header {
  display: flex;
  justify-content: flex-end;
  padding: 8px 20px 4px;
  color: var(--text-3);
  font-size: 12px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
  position: sticky;
  top: 0;
  background: var(--bg-base);
  z-index: 1;
}
.event-list { list-style: none; padding: 4px 0; }
.event-item {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 5px 20px;
  border-bottom: 1px solid var(--border);
  transition: background 0.15s;
}
.event-item:hover { background: var(--bg-hover); }
.event-item.expandable { cursor: pointer; }
.event-time {
  color: var(--text-2);
  font-size: 12px;
  min-width: 62px;
  flex-shrink: 0;
  padding-top: 1px;
}
.event-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 1px 7px;
  border-radius: var(--radius-sm);
  font-size: 11px;
  font-weight: bold;
  min-width: 62px;
  justify-content: center;
  flex-shrink: 0;
  letter-spacing: 0.3px;
}
.icon { font-style: normal; margin-right: 2px; }

.badge-user {
  background: var(--warning-soft);
  color: var(--warning);
  border: 1px solid color-mix(in srgb, var(--warning) 40%, transparent);
}
.badge-msg,
.badge-tool {
  background: var(--accent-soft);
  color: var(--accent);
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
}
.badge-tool-done {
  background: var(--success-soft);
  color: var(--success);
  border: 1px solid color-mix(in srgb, var(--success) 40%, transparent);
}
.badge-think {
  background: var(--think-soft);
  color: var(--think);
  border: 1px solid color-mix(in srgb, var(--think) 40%, transparent);
}
.badge-sys {
  background: var(--danger-soft);
  color: var(--danger);
  border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
}

.event-content { flex: 1; min-width: 0; }
.event-line {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.event-action {
  color: var(--text-1);
  font-size: 13px;
  white-space: nowrap;
  flex-shrink: 0;
}
.event-desc {
  color: var(--text-2);
  font-size: 13px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}
.event-item:hover .expand-arrow { color: var(--accent-mute); }

/* ===== 聊天记录模式（历史会话） ===== */
/* 结构：session-header（正常流）+ 内容行（聊天流 + 右侧输入列表） */
.chat-content-row {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: stretch;
}
.chat-body {
  flex: 1;
  min-height: 0;
  min-width: 0;
  overflow-y: auto;
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

/* 定位闪烁提示 */
@keyframes flash {
  0%, 100% { box-shadow: none; }
  30% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 55%, transparent); border-radius: var(--radius-lg); }
}
.flash-target { animation: flash 1.2s ease; }

/* 右侧用户输入快速定位面板（Naive UI 风格：圆角卡片列表项） */
.user-inputs-panel {
  width: 220px;
  flex-shrink: 0;
  border-left: 1px solid var(--border);
  background: var(--bg-surface);
  overflow-y: auto;
  padding: 10px 10px 12px;
  font-family: var(--font-sans);
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.user-inputs-header {
  padding: 0 4px 2px;
  font-size: 12px;
  color: var(--text-3);
  flex-shrink: 0;
}
.user-input-item {
  display: flex;
  align-items: flex-start;
  gap: 6px;
  flex-shrink: 0; /* 面板内容超出时不压缩条目，保证两行完整显示 */
  padding: 6px 10px;
  font-size: 12px;
  line-height: 1.5;
  color: var(--text-2);
  cursor: pointer;
  border: 1px solid transparent;
  border-radius: var(--radius-md);
  transition: all 0.15s;
}
.user-input-idx {
  flex-shrink: 0;
  min-width: 16px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: 11px;
  color: var(--text-3);
  padding-top: 1px;
}
.user-input-text {
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  overflow: hidden;
  min-width: 0;
}
.user-input-item:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
.user-input-item.active {
  background: var(--warning-soft);
  border-color: color-mix(in srgb, var(--warning) 40%, transparent);
  color: var(--warning);
}
.user-input-item.active .user-input-idx {
  color: var(--warning);
}

/* 聊天气泡 */
.chat-msg { display: flex; }
.chat-msg.user { justify-content: flex-end; }
.chat-msg.assistant { justify-content: flex-start; }
.chat-msg.system { justify-content: center; }
.chat-bubble {
  max-width: 78%;
  padding: 10px 14px;
  border-radius: var(--radius-lg);
  font-size: 13px;
  line-height: 1.6;
  word-break: break-word;
  white-space: pre-wrap;
}
.chat-msg.user .chat-bubble {
  background: var(--accent-soft);
  border: 1px solid color-mix(in srgb, var(--accent) 40%, transparent);
  color: var(--text-1);
  border-bottom-right-radius: var(--radius-sm);
}
.chat-msg.assistant .chat-bubble {
  background: var(--bg-code);
  border: 1px solid var(--border);
  color: var(--text-1);
  border-bottom-left-radius: var(--radius-sm);
}
.chat-msg.system .chat-bubble {
  background: transparent;
  border: 1px dashed var(--border-strong);
  color: var(--text-2);
  font-size: 12px;
  max-width: 100%;
}

/* 工具/思考折叠行 */
.chat-tool {
  display: flex;
  align-items: baseline;
  gap: 8px;
  flex-wrap: wrap;
  align-self: flex-start;
  max-width: 100%;
  padding: 6px 10px;
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background 0.15s;
}
.chat-tool:hover { background: var(--bg-hover); }
.chat-tool .event-detail { flex-basis: 100%; cursor: default; }
.chat-tool-icon { font-size: 12px; }
.chat-tool-name {
  color: var(--accent);
  font-size: 12px;
  font-weight: bold;
  flex-shrink: 0;
}
.chat-tool-desc {
  color: var(--text-2);
  font-size: 12px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
  max-width: 500px;
}
.chat-tool:hover .expand-arrow { color: var(--accent-mute); }
</style>
