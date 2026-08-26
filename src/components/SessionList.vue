<script setup lang="ts">
import type { Session } from "../types";
import { statusColor, fmtRelative } from "../types";

defineProps<{
  sessions: Session[];
  selectedId: string | null;
}>();

defineEmits<{ select: [session: Session] }>();
</script>

<template>
  <div class="group-list-panel">
    <div class="panel-header">
      <span>会话事件流</span>
      <span class="badge-count">{{ sessions.length }} 个会话</span>
    </div>
    <div
      v-for="session in sessions"
      :key="session.id"
      class="session-group"
      :class="{ active: selectedId === session.id }"
      @click="$emit('select', session)"
    >
      <div class="session-group-header">
        <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
        <span class="project-name">{{ session.project }}</span>
        <span class="session-title">{{ session.title }}</span>
        <span class="time">{{ fmtRelative(session.lastActivity) }}</span>
        <span v-if="session.unread" class="unread-indicator"></span>
      </div>
      <div class="session-group-preview">{{ session.preview }}</div>
    </div>
    <div v-if="sessions.length === 0" class="empty-state" style="padding: 40px 0;">
      <span style="font-size:48px;">📭</span>
      <span>暂无会话，等待 Claude Code 产生事件</span>
    </div>
  </div>
</template>