<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from "vue";
import type { Session, SessionStatus, View, PollConfig } from "./types";
import { getEvents, getSessions, getSessionDetail, getEventDetail, getConfig } from "./api";
import { darkTheme, type GlobalThemeOverrides } from "naive-ui";
import TitleBar from "./components/TitleBar.vue";
import EventList from "./components/EventList.vue";
import SessionList from "./components/SessionList.vue";
import SessionDetail from "./components/SessionDetail.vue";
import SettingsPanel from "./components/SettingsPanel.vue";

// Naive UI 暗色主题，主题量与 src/styles/tokens.css 保持同值（唯一样式源）
const themeOverrides: GlobalThemeOverrides = {
  common: {
    primaryColor: "#3b82f6",
    primaryColorHover: "#60a5fa",
    primaryColorPressed: "#2563eb",
    bodyColor: "#0a0f16",
    cardColor: "#111823",
    modalColor: "#1a2332",
    popoverColor: "#1a2332",
    inputColor: "#0d1420",
    borderColor: "#263445",
    borderRadius: "10px",
    borderRadiusSmall: "6px",
    fontFamily:
      "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
    fontFamilyMono:
      "'SF Mono', 'JetBrains Mono', 'Fira Code', Consolas, monospace",
  },
};

// ---- 状态 ----
const currentView = ref<View>("sessions");
const selectedSessionId = ref<string | null>(null);
// 事件流会话（hook 日志，实时）
const sessions = ref<Session[]>([]);
// 历史会话（Claude Code 原生 transcript），与事件流分开加载
const historySessions = ref<Session[]>([]);
const eventsDir = ref("");
// 选中会话的完整详情（懒加载：列表的 messages 为空，点开后单独拉取）
const sessionDetail = ref<Session | null>(null);

let pollTimer: ReturnType<typeof setInterval> | null = null;
let detailLoading = false;

// ---- 数据加载 ----
async function loadEvents() {
  try {
    // lastActivity 保留原始 ISO 时间戳，由各组件按需格式化
    const data = await getEvents();
    sessions.value = data;

    // 保持选中状态：若当前选中会话已消失则清除，否则若未选中则选最紧急的
    if (selectedSessionId.value && !sessions.value.find((s) => s.id === selectedSessionId.value)) {
      selectedSessionId.value = null;
    }
    if (!selectedSessionId.value && sessions.value.length > 0) {
      const firstUrgent =
        sessions.value.find((s) => s.status === "waiting_confirmation") ??
        sessions.value.find((s) => s.status === "error");
      selectedSessionId.value = firstUrgent ? firstUrgent.id : sessions.value[0].id;
    }
  } catch (e) {
    console.error("加载事件流失败", e);
  }
}

async function loadEventsDir() {
  try {
    const cfg = await getConfig();
    eventsDir.value = cfg.events_dir;
  } catch {
    eventsDir.value = "";
  }
}

/** 会话列表（切到历史视图时加载，不参与轮询）。 */
async function loadHistorySessions() {
  try {
    historySessions.value = await getSessions();
  } catch (e) {
    console.error("加载历史会话失败", e);
  }
}

// ---- 懒加载详情 ----
async function loadDetail(id: string) {
  if (detailLoading) return;
  detailLoading = true;
  try {
    // 数据源分开：事件流视图读 hook 日志（最新50条），历史视图读原生 transcript（全量）
    sessionDetail.value =
      currentView.value === "history-sessions"
        ? await getSessionDetail(id)
        : await getEventDetail(id);
  } catch (e) {
    console.error("加载会话详情失败", e);
    // 详情加载失败时回退到当前视图列表里的概要数据（messages 为空）
    const pool =
      currentView.value === "history-sessions" ? historySessions.value : sessions.value;
    sessionDetail.value = pool.find((s) => s.id === id) ?? null;
  } finally {
    detailLoading = false;
  }
}

// ---- 计算属性 ----
const sortedSessions = computed(() => {
  const priority: Record<SessionStatus, number> = {
    waiting_confirmation: 0,
    error: 1,
    waiting_input: 2,
    running: 3,
    idle: 4,
    completed: 5,
  };
  return [...sessions.value].sort(
    (a, b) => (priority[a.status] ?? 10) - (priority[b.status] ?? 10)
  );
});

const selectedSession = computed(() => {
  // 详情已懒加载完成则用详情，否则回退当前视图的列表概要
  if (sessionDetail.value && sessionDetail.value.id === selectedSessionId.value) {
    return sessionDetail.value;
  }
  const pool =
    currentView.value === "history-sessions" ? historySessions.value : sessions.value;
  return pool.find((s) => s.id === selectedSessionId.value) ?? null;
});

const projectGroups = computed(() => {
  const groups: Record<string, Session[]> = {};
  historySessions.value.forEach((s) => {
    if (!groups[s.project]) groups[s.project] = [];
    groups[s.project].push(s);
  });
  return Object.keys(groups).map((name) => ({ name, sessions: groups[name] }));
});

const statusCounts = computed(() => ({
  running: countByStatus("running"),
  waiting_confirmation: countByStatus("waiting_confirmation"),
  waiting_input: countByStatus("waiting_input"),
  error: countByStatus("error"),
  completed: countByStatus("completed"),
}));

// ---- 展示辅助 ----
function countByStatus(status: SessionStatus): number {
  return sessions.value.filter((s) => s.status === status).length;
}

// ---- 交互 ----
function selectSession(session: Session) {
  selectedSessionId.value = session.id;
  session.unread = false;
  // 懒加载：点开时才解析该会话的完整 jsonl
  sessionDetail.value = null;
  loadDetail(session.id);
}

// 切换视图：清空选中与详情；每次进入历史视图都重新获取列表（不参与自动刷新）
watch(currentView, (view) => {
  selectedSessionId.value = null;
  sessionDetail.value = null;
  if (view === "history-sessions") {
    loadHistorySessions();
  }
});

// ---- 生命周期 ----
onMounted(() => {
  loadEvents();
  loadEventsDir();
  // 轮询周期可配置（localStorage 持久化），设置页修改后通过事件通知重新应用
  applyPollTimer();
  window.addEventListener("ccbuddy:poll-config-changed", applyPollTimer);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
  window.removeEventListener("ccbuddy:poll-config-changed", applyPollTimer);
});

/** 读取用户配置的轮询周期（秒），默认 2。 */
function readPollConfig(): PollConfig {
  try {
    const raw = localStorage.getItem("ccbuddy.poll");
    if (raw) return JSON.parse(raw) as PollConfig;
  } catch {
    /* 配置损坏时回退默认 */
  }
  return { enabled: true, intervalSec: 2 };
}

/** 应用轮询配置：切换或停止定时器。 */
function applyPollTimer() {
  if (pollTimer) {
    clearInterval(pollTimer);
    pollTimer = null;
  }
  const cfg = readPollConfig();
  if (cfg.enabled && cfg.intervalSec > 0) {
    pollTimer = setInterval(loadEvents, cfg.intervalSec * 1000);
  }
}
</script>

<template>
  <n-config-provider :theme="darkTheme" :theme-overrides="themeOverrides" style="height:100%; display:flex; flex-direction:column;">
    <n-notification-provider placement="bottom-right">
      <n-message-provider placement="bottom">
        <TitleBar :current-view="currentView" :counts="statusCounts" @navigate="currentView = $event" />

        <div class="main-container">
          <!-- 事件流视图 -->
          <template v-if="currentView === 'sessions'">
            <EventList :sessions="sortedSessions" :selected-id="selectedSessionId" @select="selectSession" />
            <SessionDetail :session="selectedSession" empty-text="选择一个会话查看事件流" />
          </template>

          <!-- 历史会话视图（聊天记录形式） -->
          <template v-else-if="currentView === 'history-sessions'">
            <SessionList :groups="projectGroups" :selected-id="selectedSessionId" @select="selectSession" />
            <SessionDetail :session="selectedSession" mode="chat" empty-text="选择一个历史会话查看聊天记录" />
          </template>

          <!-- 设置视图 -->
          <SettingsPanel v-else-if="currentView === 'settings'" :events-dir="eventsDir" />
        </div>
      </n-message-provider>
    </n-notification-provider>
  </n-config-provider>
</template>

<style>
/* design token 见 src/styles/tokens.css（唯一样式源） */

* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  font-family: var(--font-sans);
  background: var(--bg-base);
  color: var(--text-1);
  height: 100vh;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* Vue 挂载根节点必须占满并沿纵向排列，否则内部 flex:1 无法生效 */
#app {
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

/* 主内容区：两栏 */
.main-container {
  display: flex;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

/* 左侧列表面板（事件流视图） */
.group-list-panel {
  width: 320px;
  background: var(--bg-base);
  border-right: 1px solid var(--border);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  overflow-y: auto;
}
.group-list-panel .panel-header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 14px;
  font-weight: 600;
  color: var(--text-2);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.group-list-panel .panel-header .badge-count {
  font-size: 11px;
  color: var(--text-3);
  background: var(--bg-elevated);
  padding: 2px 8px;
  border-radius: 10px;
}
.session-group {
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.15s;
}
.session-group:hover { background: var(--bg-surface); }
.session-group.active { background: var(--bg-elevated); border-left: 3px solid var(--accent); }
.session-group-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
}
.session-group-header .status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.session-group-header .project-name {
  font-size: 11px;
  color: var(--text-3);
  background: var(--bg-elevated);
  padding: 2px 6px;
  border-radius: 4px;
}
.session-group-header .session-title {
  font-size: 13px;
  font-weight: 600;
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.session-group-header .time {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}
.session-group-preview {
  padding: 0 14px 10px 30px;
  font-size: 12px;
  color: var(--text-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.unread-indicator {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  margin-left: auto;
}

/* 历史会话视图左侧：按项目分组 */
.history-group-list {
  width: 420px;
  background: var(--bg-base);
  border-right: 1px solid var(--border);
  overflow-y: auto;
  flex-shrink: 0;
}
.history-group-list .panel-header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 14px;
  font-weight: 600;
  color: var(--text-2);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.project-group {
  border-bottom: 1px solid var(--border);
}
.project-group .project-header {
  padding: 10px 16px;
  font-size: 12px;
  font-weight: 600;
  color: var(--text-2);
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border);
  position: sticky;
  top: 0;
  z-index: 1;
}
.project-group .session-item {
  padding: 8px 16px;
  cursor: pointer;
  transition: background 0.15s;
  display: flex;
  align-items: center;
  gap: 8px;
  border-left: 3px solid transparent;
}
.project-group .session-item:hover { background: var(--bg-hover); }
.project-group .session-item.active { background: var(--bg-elevated); border-left-color: var(--accent); }
.project-group .session-item .status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
.project-group .session-item .session-title {
  font-size: 13px;
  font-weight: 500;
  flex: 1;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.project-group .session-item .session-time {
  font-size: 11px;
  color: var(--text-3);
  flex-shrink: 0;
}

/* 右侧详情面板 */
.detail-panel {
  flex: 1;
  background: var(--bg-base);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
/* 滚动条 */
::-webkit-scrollbar { width: 8px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: var(--border); border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: var(--text-3); }
</style>
