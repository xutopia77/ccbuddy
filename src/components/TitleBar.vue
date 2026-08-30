<script setup lang="ts">
import type { View } from "../types";

defineProps<{
  currentView: View;
  counts: {
    running: number;
    waiting_confirmation: number;
    waiting_input: number;
    error: number;
    completed: number;
  };
}>();

defineEmits<{ navigate: [view: View] }>();

const statTags = [
  { key: "running", label: "运行中", color: "var(--success)" },
  { key: "waiting_confirmation", label: "需确认", color: "var(--warning)" },
  { key: "waiting_input", label: "等待输入", color: "var(--info)" },
  { key: "error", label: "异常", color: "var(--danger)" },
  { key: "completed", label: "已完成", color: "var(--muted)" },
] as const;

const navItems: { view: View; label: string }[] = [
  { view: "sessions", label: "事件流" },
  { view: "history-sessions", label: "历史会话" },
  { view: "settings", label: "设置" },
];
</script>

<template>
  <div class="titlebar">
    <span class="logo">CCBuddy</span>
    <span class="badge">Claude Code 会话管理器</span>
    <div class="stats-inline">
      <n-tag
        v-for="tag in statTags"
        :key="tag.key"
        round
        size="small"
        :bordered="false"
        class="stat-tag"
        :style="{ color: tag.color, background: `color-mix(in srgb, ${tag.color} 12%, transparent)` }"
        @click="$emit('navigate', 'sessions')"
      >
        <span class="dot" :style="{ background: tag.color }"></span>
        {{ tag.label }}
        <span class="num">{{ counts[tag.key] }}</span>
      </n-tag>
    </div>
    <div class="spacer"></div>
    <div class="nav-buttons">
      <n-button
        v-for="item in navItems"
        :key="item.view"
        size="small"
        secondary
        :type="currentView === item.view ? 'primary' : 'default'"
        @click="$emit('navigate', item.view)"
      >
        {{ item.label }}
      </n-button>
    </div>
  </div>
</template>

<style scoped>
.titlebar {
  height: 48px;
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  padding: 0 16px;
  gap: 12px;
  flex-shrink: 0;
  -webkit-app-region: drag;
}
.logo {
  font-weight: 700;
  font-size: 15px;
  color: var(--accent);
}
.badge {
  font-size: 11px;
  background: var(--bg-elevated);
  color: var(--text-2);
  padding: 3px 8px;
  border-radius: 20px;
  border: 1px solid var(--border);
}
.stats-inline {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: nowrap;
  -webkit-app-region: no-drag;
}
.stat-tag {
  cursor: pointer;
  font-size: 11px;
}
.stat-tag .dot {
  display: inline-block;
  width: 6px;
  height: 6px;
  border-radius: 50%;
}
.stat-tag .num { font-weight: 600; }
.spacer { flex: 1; }
.nav-buttons {
  display: flex;
  gap: 8px;
  -webkit-app-region: no-drag;
}
</style>
