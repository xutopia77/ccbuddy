<template>
  <div class="history-group-list">
    <div class="panel-header">
      <span class="panel-header-left">
        <span>{{ t("historyListTitle") }}</span>
        <span class="badge-count">{{ totalCount }} {{ t("sessionCountUnit") }}</span>
      </span>
      <!-- 手动刷新（设计稿 .refresh-btn：历史会话不自动刷新，点击重载列表与当前详情） -->
      <button
        class="refresh-btn"
        :class="{ spinning: props.refreshing }"
        :disabled="props.refreshing"
        :title="t('refreshLabel')"
        @click="$emit('refresh')"
      >
        <svg width="12" height="12" viewBox="0 0 16 16" fill="none" aria-hidden="true"><path d="M13.6 8a5.6 5.6 0 1 1-1.64-3.96" stroke="currentColor" stroke-width="1.6" stroke-linecap="round"/><path d="M13.9 1.6v3.1h-3.1" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round"/></svg>
        <span>{{ t("refreshLabel") }}</span>
      </button>
    </div>
    <!-- L-C8：Claude Code 自动清理更早的 transcript，列表据此提示 -->
    <div v-if="retentionDays !== null" class="retention-hint">
      {{ t("retentionPre") }}{{ retentionDays }}{{ t("retentionDaysUnit") }}
    </div>
    <template v-for="project in groups" :key="project.name">
      <!-- 分组头（设计稿 .proj-label）：11px 灰字 + 右侧数量 -->
      <div class="proj-label">
        <span class="proj-name">{{ project.name }}</span>
        <span class="proj-num">{{ project.sessions.length }}</span>
      </div>
      <div
        v-for="session in project.sessions"
        :key="session.id"
        class="sitem"
        :class="{ active: selectedId === session.id }"
        @click="$emit('select', session)"
      >
        <div class="sitem-top">
          <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
          <span class="sitem-title">{{ session.title }}</span>
          <span class="sitem-time">{{ fmtRelative(session.lastActivity) }}</span>
        </div>
        <div class="sitem-preview">{{ session.preview }}</div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, onMounted } from "vue";
import type { Session } from "../types";
import { statusColor, fmtRelative } from "../types";
import { getTranscriptRetention } from "../api";
import { t } from "../i18n";

const props = defineProps<{
  groups: { name: string; sessions: Session[] }[];
  selectedId: string | null;
  /** 刷新进行中（按钮 disabled + 图标旋转） */
  refreshing?: boolean;
}>();

defineEmits<{ select: [session: Session]; refresh: [] }>();

const totalCount = computed(() => props.groups.reduce((n, g) => n + g.sessions.length, 0));

// L-C8：官方 transcript 保留期（cleanupPeriodDays，只读）；
// 读不到该配置项时不显示提示（未配置时 Claude Code 用官方默认 30 天，
// 但用户显式看到 30 天却实际未配置会误导，按"读到才显示"处理）
const retentionDays = ref<number | null>(null);
onMounted(async () => {
  try {
    retentionDays.value = await getTranscriptRetention();
  } catch {
    retentionDays.value = null;
  }
});
</script>

<style scoped>
/* 左侧历史列表容器：宽度由 App.vue 内联 style（SplitDivider）控制；
   双 card 布局中外面板背景/边框由 App.vue .duo-side 提供，容器铺满 card 内部自滚 */
.history-group-list {
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
  position: sticky;
  top: 0;
  background: var(--bg-surface);
  z-index: 1;
}
.panel-header-left {
  display: flex;
  align-items: center;
  gap: 8px;
  min-width: 0;
}
/* 刷新按钮（设计稿 .refresh-btn）：小号次级 ghost，点击后图标旋转 */
.refresh-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--border-strong);
  background: var(--bg-surface);
  color: var(--text-2);
  font-size: 12px;
  font-weight: 600;
  font-family: inherit;
  padding: 4px 11px;
  border-radius: 8px;
  cursor: pointer;
  white-space: nowrap;
  flex-shrink: 0;
  transition: color 0.15s, border-color 0.15s;
}
.refresh-btn:hover { color: var(--text-1); border-color: var(--accent-mute); }
.refresh-btn:disabled { cursor: default; color: var(--text-3); }
.refresh-btn svg { display: block; }
.refresh-btn.spinning svg { animation: spin-refresh 0.8s linear infinite; }
@keyframes spin-refresh { to { transform: rotate(360deg); } }
.badge-count {
  font-size: 11px;
  color: var(--text-3);
  font-weight: 500;
  font-variant-numeric: tabular-nums;
}
.retention-hint {
  padding: 6px 16px;
  font-size: 12px;
  color: var(--text-3);
  border-bottom: 1px solid var(--border);
  background: var(--bg-surface);
}

/* 分组头（设计稿 .proj-label）：11px 灰字 + 右侧数量，sticky 跟随滚动；
   底色用 --bg-side（比列表面更深一档，区分分组分层） */
.proj-label {
  padding: 9px 14px 5px 16px;
  font-size: 11px;
  color: var(--text-3);
  letter-spacing: 0.5px;
  user-select: none;
  display: flex;
  align-items: baseline;
  gap: 6px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-side);
  position: sticky;
  top: 0;
  z-index: 1;
}
.proj-label .proj-name {
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.proj-label .proj-num {
  margin-left: auto;
  font-variant-numeric: tabular-nums;
  flex-shrink: 0;
}

/* 列表项卡片化（设计稿 .sitem），与事件流列表同形态 */
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
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
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
.sitem-preview {
  font-size: 12px;
  color: var(--text-3);
  line-height: 1.5;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  padding-left: 16px;
}
</style>
