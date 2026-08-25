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
</script>

<template>
  <div class="titlebar">
    <span class="logo">CCBuddy</span>
    <span class="badge">Claude Code 会话管理器</span>
    <div class="stats-inline">
      <span class="stat-tag running" @click="$emit('navigate', 'sessions')">
        <span class="dot"></span> 运行中 <span class="num">{{ counts.running }}</span>
      </span>
      <span class="stat-tag confirm" @click="$emit('navigate', 'sessions')">
        <span class="dot"></span> 需确认 <span class="num">{{ counts.waiting_confirmation }}</span>
      </span>
      <span class="stat-tag input" @click="$emit('navigate', 'sessions')">
        <span class="dot"></span> 等待输入 <span class="num">{{ counts.waiting_input }}</span>
      </span>
      <span class="stat-tag error" @click="$emit('navigate', 'sessions')">
        <span class="dot"></span> 异常 <span class="num">{{ counts.error }}</span>
      </span>
      <span class="stat-tag done" @click="$emit('navigate', 'sessions')">
        <span class="dot"></span> 已完成 <span class="num">{{ counts.completed }}</span>
      </span>
    </div>
    <div class="spacer"></div>
    <div class="nav-buttons">
      <button class="nav-btn" :class="{ active: currentView === 'sessions' }" @click="$emit('navigate', 'sessions')">事件流</button>
      <button class="nav-btn" :class="{ active: currentView === 'history-sessions' }" @click="$emit('navigate', 'history-sessions')">历史会话</button>
      <button class="nav-btn" :class="{ active: currentView === 'settings' }" @click="$emit('navigate', 'settings')">设置</button>
    </div>
  </div>
</template>