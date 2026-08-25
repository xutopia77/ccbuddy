<script setup lang="ts">
import { computed } from "vue";
import type { Session } from "../types";
import { statusColor } from "../types";

const props = defineProps<{
  groups: { name: string; sessions: Session[] }[];
  selectedId: string | null;
}>();

defineEmits<{ select: [session: Session] }>();

const totalCount = computed(() => props.groups.reduce((n, g) => n + g.sessions.length, 0));
</script>

<template>
  <div class="history-group-list">
    <div class="panel-header">
      <span>历史会话</span>
      <span class="badge-count">{{ totalCount }} 个会话</span>
    </div>
    <div v-for="project in groups" :key="project.name" class="project-group">
      <div class="project-header">📁 {{ project.name }}</div>
      <div
        v-for="session in project.sessions"
        :key="session.id"
        class="session-item"
        :class="{ active: selectedId === session.id }"
        @click="$emit('select', session)"
      >
        <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
        <span class="session-title">{{ session.title }}</span>
        <span class="session-time">{{ session.lastActivity }}</span>
      </div>
    </div>
  </div>
</template>