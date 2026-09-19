<template>
  <div class="detail-panel" :class="mode === 'chat' ? 'chat-mode' : 'tui'">
    <template v-if="session">
      <!-- 会话信息头：标题；次行 project 芯片 + cwd + 最后活动 + resume 命令
           （历史会话无实时状态语义，不放状态徽章） -->
      <div class="session-header">
        <div class="dm-title">
          <span class="dm-name">{{ session.title }}</span>
        </div>
        <div class="dm-sub">
          <span class="project-chip">{{ session.project }}</span>
          <span class="mono-path">{{ session.cwd || session.project }}</span>
          <span class="header-time">{{ t("lastActivityLabel") }} {{ fmtDateTime(session.lastActivity) }}</span>
          <code class="resume-cmd">{{ resumeCommand(session) }}</code>
          <n-button size="tiny" secondary class="copy-btn" @click="copyResume(session)">
            {{ copied ? t("copiedBtn") : t("copyBtn") }}
          </n-button>
        </div>
      </div>

      <!-- ===== 聊天记录模式（历史会话）===== -->
      <template v-if="mode === 'chat'">
        <div class="chat-content-row">
          <div ref="chatBodyEl" class="chat-body">
            <template v-for="(msg, idx) in session.messages" :key="idx">
              <!-- 折叠块：工具调用 / 工具输出 / 思考过程（时间槽 + 类型小标签 + 工具名 + 概要） -->
              <template v-if="isCollapsedInChat(msg)">
                <div
                  class="collapsed-line"
                  :class="{ open: expanded.has(idx), fail: isToolError(msg) }"
                  @click="toggle(idx, msg)"
                >
                  <div class="cline-head">
                    <span class="ttag" :class="badgeOf(msg).cls">{{ badgeOf(msg).label }}</span>
                    <span class="htime">{{ msg.time }}</span>
                    <span v-if="msg.toolCall" class="cname">{{ msg.toolCall }}</span>
                    <span class="arrow">{{ expanded.has(idx) ? "▾" : "▸" }}</span>
                  </div>
                  <div class="cdesc">{{ summarize(msg) }}</div>
                </div>
                <pre v-if="expanded.has(idx)" class="collapsed-detail">{{ detailOf(msg) }}</pre>
              </template>
              <!-- 用户消息：右对齐气泡（无头像），data-msg-idx 供右栏定位 -->
              <div v-else-if="msg.type === 'user'" class="brow user" :data-msg-idx="idx">
                <div class="bbox">
                  <div class="bmeta"><span class="ttag" :class="badgeOf(msg).cls">{{ badgeOf(msg).label }}</span><span class="htime">{{ msg.time }}</span></div>
                  <div class="bubble">{{ msg.content }}</div>
                </div>
              </div>
              <!-- assistant：MSG 长文本块（markdown 源文本不渲染，pre 展示） -->
              <div v-else-if="msg.type === 'assistant'" class="msg-block">
                <div class="msg-block-head">
                  <span class="ttag" :class="badgeOf(msg).cls">{{ badgeOf(msg).label }}</span>
                  <span class="htime">{{ msg.time }}</span>
                </div>
                <pre class="msg-block-body">{{ msg.content }}</pre>
              </div>
              <!-- system 系统消息：两行式（第一行 SYS 标签 + 时间，第二行描述） -->
              <div v-else class="hline" :class="{ perm: isPerm(msg) }">
                <div class="cline-head">
                  <span class="ttag" :class="badgeOf(msg).cls">{{ badgeOf(msg).label }}</span>
                  <span class="htime">{{ msg.time }}</span>
                </div>
                <div class="hdesc">{{ summarize(msg) || msg.content }}</div>
              </div>
            </template>
            <!-- M-C3：子代理转录列表（消息在独立文件，本批只读列表展示） -->
            <div v-if="(session.subagents?.length ?? 0) > 0" class="subagent-block">
              <div class="subagent-header">{{ t("subagentHeader") }} {{ session.subagents!.length }}</div>
              <div v-for="sa in session.subagents" :key="sa.id" class="subagent-item">
                <span class="subagent-name">{{ sa.id }}</span>
                <span class="subagent-desc">{{ sa.description || t("subagentNoDesc") }}</span>
              </div>
            </div>
            <div class="bottom-spacer"></div>
          </div>
          <!-- 右侧用户输入定位 card 已提升到 App.vue 三栏独立列（设计稿 .duo.triple 第三列），
               数据与 scrollToUser 定位经 defineExpose 暴露 -->
        </div>
      </template>

      <!-- ===== 事件流时间线模式（实时视图） ===== -->
      <div v-else class="event-stream">
        <div class="stream-header">{{ t("streamTitle") }} · {{ session.messages.length }} {{ t("streamCountUnit") }} · {{ t("streamHint") }}</div>

        <div class="stream-inner">
          <!-- 等待横幅：Claude 正在等你拍板时置顶一条提示。只提示不代答，
               拍板动作一律回终端（面板监控不控制），按钮只提供恢复命令复制。 -->
          <div v-if="waitMsg" class="wait-banner">
            <div class="wb-head">
              <svg class="wb-ic" viewBox="0 0 24 24" aria-hidden="true">
                <circle cx="12" cy="12" r="9" />
                <path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4.5" />
                <line x1="12" y1="17.5" x2="12.01" y2="17.5" />
              </svg>
              {{ waitTitle }}
              <span class="wb-since">· {{ fmtRelative(session.lastActivity) }}</span>
              <span class="wb-src">{{ waitMsg.toolCall || waitMsg.type }} · {{ waitMsg.time }}</span>
            </div>
            <p class="wb-body">{{ summarize(waitMsg, 220) }}</p>
            <div class="wb-go">
              <n-button size="small" type="warning" @click="copyResume(session)">
                {{ copied ? t("copiedBtn") : t("waitGoBtn") }}
              </n-button>
              <span class="wb-hint">{{ t("waitGoHint") }}<code>{{ resumeCommand(session) }}</code></span>
            </div>
          </div>

          <ul class="event-list">
            <li
              v-for="(msg, idx) in eventMessages"
              :key="idx"
              class="evrow"
              :class="[kindOf(msg), { expandable: hasDetail(msg) }]"
              @click="toggle(idx, msg)"
            >
              <span class="ic" aria-hidden="true">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" v-html="iconOf(msg)"></svg>
              </span>
              <span class="hd">
                <span class="nm">{{ nameOf(msg) }}</span>
                <span class="tx">{{ txOf(msg) }}</span>
                <span class="tm">{{ msg.time }}</span>
              </span>
              <pre v-if="expanded.has(idx)" class="collapsed-detail">{{ detailOf(msg) }}</pre>
            </li>
          </ul>
        </div>
        <div class="bottom-spacer"></div>
      </div>
    </template>
    <n-empty
      v-else
      :description="emptyText || t('detailEmptyText')"
      style="margin: auto"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, watch, computed } from "vue";
import type { Message, Session } from "../types";
import { fmtDateTime, fmtRelative } from "../types";
import { t } from "../i18n";

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
  label: string;
  cls: string;
  action: string;
}

/** M-C4：tool_result 内容带失败标记（后端 is_error=true 时加此前缀） */
const TOOL_ERROR_PREFIX = "[工具执行失败]";

/** 事件流里后端固定生成的失败系统消息文案（events_mgr::system_message 调用点） */
const SYS_FAIL_TEXTS = ["工具调用失败", "会话停止失败"];

/**
 * 失败判定，两条来源：
 * - 历史会话（原生转录）：tool_result 内容带 is_error 前缀；
 * - 事件流（实时）：后端 postToolUseFailure / stopFailure 落成 system 消息，内容为固定文案。
 */
function isToolError(msg: Message): boolean {
  if (msg.type === "tool_result") return msg.content.startsWith(TOOL_ERROR_PREFIX);
  return msg.type === "system" && SYS_FAIL_TEXTS.some((s) => msg.content.includes(s));
}

/** PermissionRequest 判定：system 消息且内容提到"权限/确认"，但排除"权限被拒"
 * （后端状态机对 PermissionRequest 生成"请求权限/等待你确认"类 system 消息，
 * PermissionDenied 生成的"权限被拒"同含"权限"但已无需用户拍板，故排除；
 * 与 isToolError 同款简单前缀/关键字判定风格） */
function isPerm(msg: Message): boolean {
  return (
    msg.type === "system" && /权限|确认/.test(msg.content) && !msg.content.includes("被拒")
  );
}

function badgeOf(msg: Message): BadgeInfo {
  switch (msg.type) {
    case "user":
      return { label: "USER", cls: "badge-user", action: t("actUser") };
    case "assistant":
      return { label: "MSG", cls: "badge-msg", action: t("actClaude") };
    case "thinking":
      return { label: "THINK", cls: "badge-think", action: t("actThink") };
    case "tool_use":
      return { label: "TOOL", cls: "badge-tool", action: t("actTool") };
    case "tool_result":
      // M-C4：失败结果用错误徽标（红底），正常结果保持完成样式
      if (isToolError(msg)) {
        return { label: "TOOL", cls: "badge-tool-error", action: t("actToolFail") };
      }
      return { label: "DONE", cls: "badge-tool-done", action: t("actToolDone") };
    default:
      return { label: "SYS", cls: "badge-sys", action: t("actSys") };
  }
}

/** 事件行图标：24×24 线性图标，路径取自 tmp/ui-example-event.html 的 <symbol> 定义 */
const ICON_USER =
  '<path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2"/><circle cx="12" cy="7" r="4"/>';
const ICON_BOT =
  '<rect x="4" y="8" width="16" height="12" rx="2"/><line x1="9" y1="13.5" x2="9.01" y2="13.5"/><line x1="15" y1="13.5" x2="15.01" y2="13.5"/><path d="M12 8V4"/><line x1="12" y1="3" x2="12.01" y2="3"/>';
const ICON_THINK =
  '<circle cx="12" cy="12" r="9"/><path d="M9.1 9a3 3 0 0 1 5.8 1c0 2-3 2.5-3 4.5"/><line x1="12" y1="17.5" x2="12.01" y2="17.5"/>';
const ICON_TOOL =
  '<path d="M14.7 6.3a1 1 0 0 0 0 1.4l1.6 1.6a1 1 0 0 0 1.4 0l3.77-3.77a6 6 0 0 1-7.94 7.94l-6.91 6.91a2.12 2.12 0 0 1-3-3l6.91-6.91a6 6 0 0 1 7.94-7.94l-3.76 3.76z"/>';
const ICON_WAIT = '<circle cx="12" cy="12" r="9"/><polyline points="12 7 12 12 15.5 14"/>';
const ICON_ERR =
  '<circle cx="12" cy="12" r="9"/><line x1="9" y1="9" x2="15" y2="15"/><line x1="15" y1="9" x2="9" y2="15"/>';
const ICON_SYS =
  '<circle cx="12" cy="12" r="9"/><line x1="12" y1="11" x2="12" y2="16.5"/><line x1="12" y1="7.8" x2="12.01" y2="7.8"/>';

/** 配色分类 → 图标（一一对应，改 kindOf 时此处同步） */
const KIND_ICON: Record<string, string> = {
  "k-user": ICON_USER,
  "k-bot": ICON_BOT,
  "k-think": ICON_THINK,
  "k-tool": ICON_TOOL,
  "k-wait": ICON_WAIT,
  "k-err": ICON_ERR,
  "k-sys": ICON_SYS,
};

/** 事件行配色分类（ui-example-event：用户蓝 / Claude 紫 / 思考灰 / 工具最安静 / 失败红 / 等待琥珀） */
function kindOf(msg: Message): string {
  if (isToolError(msg)) return "k-err";
  if (isPerm(msg)) return "k-wait";
  // 工具行：事件流里后端把工具调用记成 assistant + toolCall（content 是"调用工具 X…"），
  // 光看 type 会跟 Claude 正文混在一起，必须先按 toolCall 摘出来
  if (msg.toolCall) return "k-tool";
  switch (msg.type) {
    case "user":
      return "k-user";
    case "assistant":
      return "k-bot";
    case "thinking":
      return "k-think";
    case "tool_use":
    case "tool_result":
      return "k-tool";
    default:
      return "k-sys";
  }
}

function iconOf(msg: Message): string {
  return KIND_ICON[kindOf(msg)] ?? ICON_SYS;
}

/** 事件行名称：工具行显示工具名（Grep/Read/Bash…），失败行显示失败文案，其余用动作文案 */
function nameOf(msg: Message): string {
  if (msg.toolCall) return msg.toolCall;
  if (isToolError(msg)) return summarize(msg, 40) || t("actToolFail");
  return badgeOf(msg).action;
}

/**
 * 事件行描述。工具行只显示入参概要——工具名已单独显示，content 首行"调用工具 X"是重复的。
 * 概要由后端提取并限长（见 events_mgr::tool_brief），这里直接展示。
 * 失败行的失败文案已用作名称，故不再补描述。
 */
function txOf(msg: Message): string {
  if (isToolError(msg)) return "";
  if (msg.toolCall) return toolBody(msg);
  return summarize(msg);
}

/** 等待横幅：会话处于等待态时取最后一条消息作为正文（列表倒序前取，故放在 eventMessages 之外） */
const waitMsg = computed<Message | null>(() => {
  const s = props.session;
  if (!s) return null;
  if (s.status !== "waiting_confirmation" && s.status !== "waiting_input") return null;
  return s.messages.length ? s.messages[s.messages.length - 1] : null;
});

const waitTitle = computed(() =>
  props.session?.status === "waiting_confirmation" ? t("waitTitleConfirm") : t("waitTitleInput")
);

/** tool_use 内容首行为"调用工具 X"，拆出剩余入参 */
function toolBody(msg: Message): string {
  const idx = msg.content.indexOf("\n");
  return idx === -1 ? "" : msg.content.slice(idx + 1);
}

const SUMMARY_KEYS = [
  "file_path", "command", "url", "pattern", "path", "description", "query", "content",
];

/** 压成一行并按 limit 截断（超出以 ... 收尾） */
function clip(s: string, limit: number): string {
  const t = s.replace(/\s+/g, " ").trim();
  return t.length > limit ? t.slice(0, limit) + "..." : t;
}

/** 工具入参概要：从 JSON 入参中取最能描述意图的字段，取不到则回退首行 */
function inputBrief(body: string, limit: number): string {
  if (!body) return "";
  try {
    const input = JSON.parse(body) as Record<string, unknown>;
    for (const k of SUMMARY_KEYS) {
      if (typeof input[k] === "string" && input[k]) return clip(String(input[k]), limit);
    }
  } catch {
    // 非 JSON 入参，直接取首行
  }
  return clip(body.split("\n")[0] || "", limit);
}

/** 生成一行概要（不展示详细内容）；n 为截断长度 */
function summarize(msg: Message, n = 90): string {
  if (msg.type === "tool_use") {
    return inputBrief(toolBody(msg), n) || msg.toolCall || "";
  }
  // M-C4：失败结果概要去掉标记前缀（徽标已表达失败态）
  if (isToolError(msg)) {
    return clip(msg.content.slice(TOOL_ERROR_PREFIX.length).trim(), n);
  }
  return clip(msg.content, n);
}

/** 是否有可展开的详情 */
function hasDetail(msg: Message): boolean {
  if (msg.type === "thinking") return true;
  if (msg.type === "tool_result") return !!msg.content.trim();
  // 历史会话的 tool_use 带完整入参，值得展开；事件流工具行只有一行概要，展开无新内容
  if (msg.type === "tool_use") return !!toolBody(msg).trim();
  if (msg.toolCall) return false;
  return msg.content.length > 90;
}

/** 详情全文 */
function detailOf(msg: Message): string {
  return msg.type === "tool_use" ? toolBody(msg) : msg.content;
}

/** 事件流时间线：最新事件在最上方（倒序）。历史聊天记录保持正序，故仅此处使用。 */
const eventMessages = computed(() =>
  props.session ? [...props.session.messages].reverse() : []
);

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

/** 点击某条用户输入：滚动定位到聊天流中对应的消息（由右栏 card 调用，经 defineExpose 暴露） */
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

/** 右栏用户输入 card 已提升到 App.vue 三栏独立列：定位与数据经 expose 供其调用/读取 */
defineExpose({
  scrollToUser,
  activeUserIdx,
  summarize,
  userInputs,
});
</script>

<style scoped>
/* ===== 公共：面板基调（design token，见 styles/tokens.css） ===== */
.detail-panel {
  position: relative; /* chat 模式的 session-header 绝对定位锚点 */
  background: var(--bg-surface);
  font-family: var(--font-sans);
  font-size: 13px;
  line-height: 1.6;
  min-width: 0;
  margin: 0;
}

/* ===== 会话信息头（设计稿 .duo-main-head + .big-badge）：无独立背景，
   继承卡片 panel-2 底，仅 border-bottom 分隔（设计稿如此） ===== */
.session-header {
  padding: 14px 20px 12px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.dm-title {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  min-width: 0;
}
.dm-name {
  font-size: 16px;
  font-weight: 700;
  letter-spacing: -0.01em;
  line-height: 1.4;
  color: var(--text-1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 200px;
}
.dm-sub {
  display: flex;
  align-items: center;
  gap: 12px;
  flex-wrap: wrap;
  font-size: 12px;
  color: var(--text-3);
  min-width: 0;
}
.project-chip {
  font-size: 10.5px;
  color: var(--text-3);
  background: var(--bg-elevated);
  border: 1px solid var(--border);
  padding: 1px 8px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
  white-space: nowrap;
}
.mono-path {
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--text-3);
  word-break: break-all;
  min-width: 0;
}
.header-time {
  color: var(--text-3);
  font-size: 12px;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
.resume-cmd {
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--text-2);
  background: var(--bg-code);
  border: 1px solid var(--border);
  padding: 2px 8px;
  border-radius: 5px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 120px;
  user-select: text;
}
.copy-btn {
  /* Naive 按钮继承面板字体，保持命令行观感 */
  font-family: var(--font-mono);
  font-size: 12px;
  flex-shrink: 0;
}

/* ===== 展开详情块（事件流 / 聊天两种模式共用，设计稿 .collapsed-detail） =====
   完全展开不设内滚上限：内容多高就多高，跟随外层聊天流整体滚动 */
.collapsed-detail {
  font-family: var(--font-mono);
  font-size: 12.5px;
  line-height: 1.65;
  color: var(--text-2);
  background: var(--bg-code);
  border: 1px solid var(--border);
  border-radius: var(--radius-inset);
  padding: 12px 14px;
  margin: -6px 0 0 12px;
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
  cursor: default;
}
/* 聊天模式：展开详情铺满整行（无左侧缩进） */
.collapsed-line + .collapsed-detail {
  margin: 2px 0 0;
}
.bottom-spacer { height: 40px; flex-shrink: 0; }

/* ===== 事件流时间线模式（设计稿 .evrow：18px 图标格 + 描述 + mono 时间） ===== */
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
  background: var(--bg-elevated);
  z-index: 1;
}
.event-list { list-style: none; padding: 4px 0; margin: 0; }
/* 事件流正文限宽居中（ui-example-event 的 .wrap），宽屏下不铺满整行 */
.stream-inner {
  width: 100%;
  max-width: 920px;
  margin: 0 auto;
  padding: 0 20px;
}
.evrow {
  display: flex;
  align-items: flex-start;
  gap: 11px;
  padding: 7px 10px;
  border-radius: var(--radius-md);
  flex-wrap: wrap;
  transition: background 0.15s;
}
.evrow:hover { background: var(--bg-hover); }
.evrow.expandable { cursor: pointer; }
/* 图标线性风格：无边框无底色，颜色随分类（ui-example-event 的 .ic——
   fill:none + stroke:currentColor，故路径本身不带描边属性） */
.evrow .ic {
  width: 14px;
  height: 14px;
  margin-top: 4px;
  flex-shrink: 0;
  display: inline-flex;
  color: var(--text-3);
}
.evrow .ic svg {
  display: block;
  stroke: currentColor;
  fill: none;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.evrow .hd {
  flex: 1;
  min-width: 0;
  display: flex;
  align-items: baseline;
  gap: 8px;
  font-size: 12.5px;
  line-height: 1.6;
}
.evrow .nm { font-weight: 600; color: var(--text-2); flex-shrink: 0; }
.evrow .tx {
  flex: 1;
  min-width: 0;
  color: var(--text-3);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.evrow .tm {
  font-size: 11px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono);
  flex-shrink: 0;
  white-space: nowrap;
}
/* 展开详情在行内换行铺满（完全展开，无内滚），左缩进对齐描述文字 */
.evrow .collapsed-detail {
  flex-basis: 100%;
  margin: 4px 0 6px 25px;
}
/* ---- 分类配色：颜色只落在图标与名称上，描述/时间保持文字色 ---- */
.evrow.k-user .ic,
.evrow.k-user .nm { color: var(--info); }
.evrow.k-bot .ic,
.evrow.k-bot .nm { color: var(--think); }
/* 思考：Claude 的自言自语，压到灰调 */
.evrow.k-think .ic,
.evrow.k-think .nm { color: var(--muted); }
/* 工具调用：最安静的一类。示例里工具行用比正文灰更暗一档的 faint 色且不加粗，
   这里在 --text-3 基础上再压一档（不新增色值），让工具行整体退到背景里去 */
.evrow.k-tool .ic,
.evrow.k-tool .nm {
  color: color-mix(in srgb, var(--text-3) 70%, transparent);
}
.evrow.k-tool .nm { font-weight: 500; }
/* 失败事件（isToolError）：淡红底一行 */
.evrow.k-err { background: var(--danger-soft); }
.evrow.k-err .ic,
.evrow.k-err .nm { color: var(--danger); }
/* PermissionRequest：琥珀底一行（只展示，拍板一律回终端，见顶部等待横幅） */
.evrow.k-wait { background: var(--warning-soft); }
.evrow.k-wait .ic,
.evrow.k-wait .nm { color: var(--warning); }
.evrow.k-wait .tx { color: var(--text-2); }

/* ===== 等待横幅（ui-example-event 的 .banner）：仅等待态出现 ===== */
.wait-banner {
  margin-top: 16px;
  background: var(--warning-soft);
  border: 1px solid color-mix(in srgb, var(--warning) 35%, transparent);
  border-left: 3px solid var(--warning);
  border-radius: var(--radius-lg);
  padding: 13px 16px;
}
.wb-head {
  display: flex;
  align-items: center;
  gap: 8px;
  font-weight: 600;
  color: var(--warning);
  flex-wrap: wrap;
}
.wb-ic {
  width: 15px;
  height: 15px;
  flex: none;
  stroke: currentColor;
  fill: none;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.wb-since { font-weight: 400; font-size: 12px; opacity: 0.85; }
.wb-src {
  margin-left: auto;
  font-weight: 400;
  font-size: 11px;
  color: var(--text-3);
  font-family: var(--font-mono);
}
.wb-body {
  margin: 7px 0 10px;
  font-size: 13px;
  line-height: 1.6;
  color: var(--text-1);
  word-break: break-word;
}
.wb-go {
  display: flex;
  align-items: center;
  gap: 10px;
  flex-wrap: wrap;
}
.wb-hint { font-size: 12px; color: var(--text-3); }
.wb-hint code {
  font-family: var(--font-mono);
  font-size: 11.5px;
  color: var(--text-2);
}

/* ===== 类型小标签（设计稿 .ttag：聊天流 / 折叠行共用） ===== */
.ttag {
  font-size: 10.5px;
  font-weight: 700;
  letter-spacing: 0.4px;
  padding: 1px 7px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
  white-space: nowrap;
}
.badge-user { background: var(--info-soft); color: var(--info); }
.badge-msg { background: var(--success-soft); color: var(--success); }
.badge-think { background: var(--muted-soft); color: var(--text-2); }
.badge-tool { background: var(--accent-soft); color: var(--accent); }
.badge-tool-done { background: var(--success-soft); color: var(--success); }
/* M-C4：失败的工具结果 */
.badge-tool-error { background: var(--danger-soft); color: var(--danger); }
.badge-sys { background: var(--muted-soft); color: var(--text-3); }

/* ===== 聊天记录模式（历史会话，设计稿 .duo.triple 三栏方案） =====
   结构：session-header + 内容行（聊天流 + 右侧输入列表 card）
   聊天流自身是滚动容器（随窗口高度滚），右侧输入面板独立列、独立滚动，
   两栏滚动互不牵连——中栏再长也不拖动右栏，右栏点定位滚动的是聊天流 */
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

/* 行首时间槽：完整本地时间 YYYY-MM-DD HH:MM:SS，等宽字体不再固定宽，按内容自适应 */
.htime {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
  font-family: var(--font-mono);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}

/* 折叠行（工具调用 / 工具输出 / 思考过程）：两行式——
   第一行 标签 + 时间（+ 工具名）+ 箭头，第二行 概要 */
.collapsed-line {
  display: flex;
  flex-direction: column;
  gap: 3px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-left: 2px solid var(--text-3); /* 默认灰，open 珊瑚 / fail 猩红（对齐设计稿） */
  border-radius: var(--radius-inset);
  background: var(--bg-surface);
  cursor: pointer;
  user-select: none;
  font-size: 12.5px;
  line-height: 1.55;
  transition: border-color 0.15s, background 0.15s;
}
.collapsed-line:hover {
  border-color: var(--border-strong);
  background: var(--bg-elevated); /* panel-3 微亮，不抢内容（设计稿同款） */
}
.collapsed-line.open { border-left-color: var(--accent); }
.collapsed-line.fail { border-left-color: var(--danger); }
/* 第一行：标识 + 时间 + 工具名 + 箭头（横向） */
.cline-head {
  display: flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
}
.cline-head .cname {
  font-family: var(--font-mono);
  font-size: 11.5px;
  font-weight: 600;
  color: var(--text-1);
  flex-shrink: 0;
  white-space: nowrap;
}
.cline-head .arrow {
  margin-left: auto;
  flex-shrink: 0;
  color: var(--text-3);
  font-size: 11px;
  line-height: 1;
}
.collapsed-line .cdesc {
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
}
.collapsed-line.fail .cdesc { color: var(--danger); }

/* assistant：MSG 长文本块（左玉翠条 + 头部时间槽/标签 + pre 正文） */
.msg-block {
  padding: 10px 14px;
  border: 1px solid var(--border);
  border-left: 2px solid var(--success);
  border-radius: var(--radius-inset);
  background: var(--bg-surface);
}
.msg-block-head {
  display: flex;
  align-items: baseline;
  gap: 9px;
  margin-bottom: 6px;
  font-variant-numeric: tabular-nums;
}
.msg-block-body {
  font-family: var(--font-sans);
  font-size: 12.5px;
  line-height: 1.7;
  color: var(--text-2);
  white-space: pre-wrap;
  word-break: break-word;
  user-select: text;
  margin: 0;
}

/* user：右对齐气泡（--bg-elevated 底，右上角 4px） */
.brow { display: flex; }
.brow.user { justify-content: flex-end; }
.bbox {
  max-width: 76%;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.brow.user .bbox { align-items: flex-end; }
.bmeta {
  font-size: 11px;
  color: var(--text-3);
  display: flex;
  gap: 8px;
  align-items: center;
  font-variant-numeric: tabular-nums;
}
.bubble {
  display: inline-block;
  text-align: left;
  padding: 8px 13px;
  border-radius: var(--radius-card); /* 设计稿 user 气泡用卡片级圆角 */
  border-top-right-radius: var(--radius-sm);
  font-size: 13px;
  line-height: 1.6;
  word-break: break-word;
  max-width: 100%;
  white-space: pre-wrap;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  color: var(--text-1);
}

/* system 系统消息：两行式（第一行 SYS 标签 + 时间，第二行描述）；权限确认类着珊瑚 */
.hline {
  display: flex;
  flex-direction: column;
  gap: 2px;
  padding: 4px 2px;
  font-size: 12.5px;
  line-height: 1.55;
}
.hline .hdesc {
  color: var(--text-2);
  min-width: 0;
}
.hline.perm .hdesc { color: var(--warning); }

/* 定位闪烁提示 */
@keyframes flash {
  0%, 100% { box-shadow: none; }
  30% { box-shadow: 0 0 0 3px color-mix(in srgb, var(--accent) 55%, transparent); border-radius: var(--radius-lg); }
}
.flash-target { animation: flash 1.2s ease; }

/* 右栏用户输入定位 card 样式已随结构提升移至 App.vue（三栏独立列渲染） */
/* 右栏用户输入定位条目样式：已随结构提升移至 App.vue 全局样式段 */

/* M-C3：子代理转录列表（消息流末尾只读展示） */
.subagent-block {
  align-self: flex-start;
  max-width: 100%;
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-md);
  padding: 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 4px;
  background: var(--bg-surface);
}
.subagent-header {
  font-size: 12px;
  font-weight: 700;
  color: var(--text-2);
}
.subagent-item {
  display: flex;
  align-items: baseline;
  gap: 8px;
  min-width: 0;
}
.subagent-name {
  font-size: 11px;
  color: var(--text-3);
  font-family: var(--font-mono);
  flex-shrink: 0;
}
.subagent-desc {
  font-size: 12px;
  color: var(--text-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  min-width: 0;
  max-width: 500px;
}
</style>
