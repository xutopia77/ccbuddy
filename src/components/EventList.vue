<template>
  <div class="group-list-panel">
    <div class="panel-header">
      <span>{{ t("eventListTitle") }}</span>
      <span class="badge-count">{{ sessions.length }} {{ t("sessionCountUnit") }}</span>
    </div>
    <!-- 列表已按状态排序（需确认 > 异常 > 等待输入 > 运行中 > 空闲 > 已完成），视觉跟着 status 走 -->
    <div
      v-for="session in sessions"
      :key="session.id"
      class="sitem"
      :class="[{ active: selectedId === session.id }, attentionCls(session)]"
      @click="$emit('select', session)"
    >
      <!-- 第一行：状态文字标签（六态软色底，事件流靠状态驱动）+ 会话名 + 未读点 + 活跃时间 -->
      <div class="sitem-top">
        <span class="status-chip" :class="dotCls(session.status)">{{ statusLabel(session.status) }}</span>
        <span class="sitem-title">{{ session.title }}</span>
        <span v-if="session.unread && !isAcked(session)" class="unread-dot" :title="t('unreadTitle')"></span>
        <span class="sitem-time">{{ fmtRelative(session.lastActivity) }}</span>
      </div>
      <!-- 第二行：最新消息预览（需确认/异常着色）+ 消息数量（靠右弱化）。cwd 不进列表——身份确认在右侧详情头 -->
      <div class="sitem-info">
        <span class="sitem-preview">{{ session.preview }}</span>
        <span class="sitem-count">{{ t("msgCountPre") }} {{ session.messages.length }}</span>
      </div>
    </div>
    <n-empty
      v-if="sessions.length === 0"
      :description="t('eventsListEmpty')"
      style="padding: 48px 0"
    />
  </div>
</template>

<script setup lang="ts">
import type { Session } from "../types";
import { dotCls, statusLabel, fmtRelative } from "../types";
import { t } from "../i18n";
import { isAcked } from "../ack";

defineProps<{
  sessions: Session[];
  selectedId: string | null;
}>();

defineEmits<{ select: [session: Session] }>();

/**
 * 需关注高亮类：底色/文字色跟状态标签同色系。
 * 已确认过（用户点开看过）的不再高亮——否则等待类/异常会话会一直"叫"下去；
 * 有新事件（lastActivity 变化）后 isAcked 自动失效，重新提醒。
 */
function attentionCls(session: Session): string {
  if (!session.unread || isAcked(session)) return "";
  if (session.status === "error") return "attention error";
  if (session.status === "waiting_input") return "attention input";
  return "attention confirm";
}
</script>

<style scoped>
/* 左侧会话列表容器：宽度由 App.vue 内联 style（SplitDivider）控制；
   双 card 布局中外面板背景/边框由 App.vue .duo-side 提供，容器铺满 card 内部自滚 */
.group-list-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  overflow-y: auto;
  min-width: 0;
}
.panel-header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 13px;
  font-weight: 700;
  color: var(--text-2);
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-shrink: 0;
  position: sticky;
  top: 0;
  background: var(--bg-surface);
  z-index: 1;
}
.badge-count {
  font-size: 11px;
  color: var(--text-3);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}

/* 列表项卡片化（设计稿 .sitem）：第一行状态点+标题+未读点+相对时间，第二行 preview */
.sitem {
  position: relative;
  padding: 10px 14px 9px 16px;
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.15s;
  display: flex;
  flex-direction: column;
  gap: 3px;
}
.sitem:hover { background: var(--bg-hover); }
.sitem.active { background: var(--bg-elevated); }
.sitem.active::before {
  content: "";
  position: absolute;
  left: 0;
  top: 8px;
  bottom: 8px;
  width: 3px;
  border-radius: 0 3px 3px 0;
  background: var(--accent);
}
.sitem-top {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
.sitem-title {
  flex: 1;
  min-width: 0;
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  line-height: 1.45;
}
.sitem-time {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
/* 状态文字标签（设计稿 .sitem-status：六态软色底替代小圆点，事件流靠状态驱动一眼可辨） */
.status-chip {
  font-size: 10px;
  font-weight: 700;
  letter-spacing: 0.3px;
  padding: 1px 7px;
  border-radius: var(--radius-sm);
  flex-shrink: 0;
  white-space: nowrap;
  line-height: 1.6;
}
.status-chip.running { background: var(--success-soft); color: var(--success); }
.status-chip.confirm { background: var(--warning-soft); color: var(--warning); }
.status-chip.input { background: var(--info-soft); color: var(--info); }
.status-chip.error { background: var(--danger-soft); color: var(--danger); }
.status-chip.done { background: var(--muted-soft); color: var(--muted); }
.status-chip.idle { background: var(--bg-hover); color: var(--text-3); }
/* 第二行：预览为主体（flex:1 单行 ellipsis）+ 消息数量靠右弱化。
   padding-left: 0——第一行以状态标签开头，第二行从行首对齐（区别于历史列表的 16px 缩进） */
.sitem-info {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 8px;
  min-width: 0;
  font-size: 11px;
  color: var(--text-3);
  line-height: 1.5;
  white-space: nowrap;
}
.sitem-info .sitem-preview {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  color: var(--text-3);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.sitem-count {
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}

/* attention 高亮：底色 tint 渐变 + 标题/预览同色，颜色与状态标签一致。
   需确认=珊瑚 / 等待输入=蓝 / 异常=猩红；用户点开确认后该类不再输出（见 attentionCls）。 */
.sitem.attention {
  background: linear-gradient(90deg, var(--warning-soft), transparent 72%);
}
.sitem.attention .sitem-title,
.sitem.attention .sitem-preview { color: var(--warning); }
.sitem.attention.input {
  background: linear-gradient(90deg, var(--info-soft), transparent 72%);
}
.sitem.attention.input .sitem-title,
.sitem.attention.input .sitem-preview { color: var(--info); }
.sitem.attention.error {
  background: linear-gradient(90deg, var(--danger-soft), transparent 72%);
}
.sitem.attention.error .sitem-title,
.sitem.attention.error .sitem-preview { color: var(--danger); }
/* 选中态：渐变末端接卡片底色，避免与 .sitem.active 的背景打架 */
.sitem.attention.active {
  background: linear-gradient(90deg, var(--warning-soft), var(--bg-elevated) 72%);
}
.sitem.attention.input.active {
  background: linear-gradient(90deg, var(--info-soft), var(--bg-elevated) 72%);
}
.sitem.attention.error.active {
  background: linear-gradient(90deg, var(--danger-soft), var(--bg-elevated) 72%);
}

/* 未读点：6px 珊瑚带辉光 */
.unread-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  flex-shrink: 0;
  box-shadow: 0 0 6px color-mix(in srgb, var(--accent) 60%, transparent);
}
</style>
