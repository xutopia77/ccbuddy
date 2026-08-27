<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import type { Session, SessionStatus, View } from "./types";
import { getSessions, getEventsDir } from "./api";
import TitleBar from "./components/TitleBar.vue";
import SessionList from "./components/SessionList.vue";
import HistoryList from "./components/HistoryList.vue";
import SessionDetail from "./components/SessionDetail.vue";
import SettingsPanel from "./components/SettingsPanel.vue";

// ---- 状态 ----
const currentView = ref<View>("sessions");
const selectedSessionId = ref<string | null>(null);
const sessions = ref<Session[]>([]);
const eventsDir = ref("");

let pollTimer: ReturnType<typeof setInterval> | null = null;

// ---- 数据加载 ----
async function loadSessions() {
  try {
    // lastActivity 保留原始 ISO 时间戳，由各组件按需格式化
    const data = await getSessions();
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
    console.error("加载会话失败", e);
  }
}

async function loadEventsDir() {
  try {
    eventsDir.value = await getEventsDir();
  } catch {
    eventsDir.value = "";
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

const selectedSession = computed(
  () => sessions.value.find((s) => s.id === selectedSessionId.value) ?? null
);

const projectGroups = computed(() => {
  const groups: Record<string, Session[]> = {};
  sessions.value.forEach((s) => {
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
}

// ---- 生命周期 ----
onMounted(() => {
  loadSessions();
  loadEventsDir();
  pollTimer = setInterval(loadSessions, 2000);
});

onUnmounted(() => {
  if (pollTimer) clearInterval(pollTimer);
});
</script>

<template>
  <TitleBar :current-view="currentView" :counts="statusCounts" @navigate="currentView = $event" />

  <div class="main-container">
    <!-- 事件流视图 -->
    <template v-if="currentView === 'sessions'">
      <SessionList :sessions="sortedSessions" :selected-id="selectedSessionId" @select="selectSession" />
      <SessionDetail :session="selectedSession" empty-text="选择一个会话查看事件流" />
    </template>

    <!-- 历史会话视图（聊天记录形式） -->
    <template v-else-if="currentView === 'history-sessions'">
      <HistoryList :groups="projectGroups" :selected-id="selectedSessionId" @select="selectSession" />
      <SessionDetail :session="selectedSession" mode="chat" empty-text="选择一个历史会话查看聊天记录" />
    </template>

    <!-- 设置视图 -->
    <SettingsPanel v-else-if="currentView === 'settings'" :events-dir="eventsDir" />
  </div>
</template>

<style>
:root {
  --bg-primary: #0a0f16;
  --bg-secondary: #111823;
  --bg-tertiary: #1a2332;
  --bg-hover: #212d3f;
  --border: #263445;
  --text-primary: #e2e8f0;
  --text-secondary: #94a3b8;
  --text-muted: #64748b;
  --accent: #3b82f6;
  --accent-hover: #60a5fa;
  --green: #10b981;
  --orange: #f59e0b;
  --red: #ef4444;
  --blue: #3b82f6;
  --gray: #6b7280;
  --radius: 10px;
  --radius-sm: 6px;
  --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif;
  --font-mono: 'SF Mono', 'JetBrains Mono', 'Fira Code', Consolas, monospace;
}

* { margin: 0; padding: 0; box-sizing: border-box; }
body {
  font-family: var(--font-sans);
  background: var(--bg-primary);
  color: var(--text-primary);
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

/* 顶部标题栏 */
.titlebar {
  height: 48px;
  background: var(--bg-secondary);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  padding: 0 16px;
  gap: 12px;
  flex-shrink: 0;
  -webkit-app-region: drag;
}
.titlebar .logo { font-weight: 700; font-size: 15px; color: var(--accent); }
.titlebar .badge {
  font-size: 11px;
  background: var(--bg-tertiary);
  color: var(--text-secondary);
  padding: 3px 8px;
  border-radius: 20px;
  border: 1px solid var(--border);
}
.titlebar .stats-inline {
  display: flex;
  gap: 8px;
  align-items: center;
  flex-wrap: nowrap;
  -webkit-app-region: no-drag;
}
.stat-tag {
  display: flex;
  align-items: center;
  gap: 4px;
  font-size: 11px;
  padding: 3px 8px;
  border-radius: 12px;
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
  color: var(--text-secondary);
  cursor: pointer;
  white-space: nowrap;
}
.stat-tag .dot { width: 6px; height: 6px; border-radius: 50%; }
.stat-tag.running .dot { background: var(--green); }
.stat-tag.confirm .dot { background: var(--orange); }
.stat-tag.input .dot { background: var(--blue); }
.stat-tag.error .dot { background: var(--red); }
.stat-tag.done .dot { background: var(--gray); }
.stat-tag .num { font-weight: 600; }
.titlebar .spacer { flex: 1; }
.titlebar .nav-buttons {
  display: flex;
  gap: 8px;
  -webkit-app-region: no-drag;
}
.nav-btn {
  background: none;
  border: none;
  color: var(--text-muted);
  cursor: pointer;
  font-size: 12px;
  padding: 6px 10px;
  border-radius: var(--radius-sm);
  transition: all 0.15s;
}
.nav-btn:hover { color: var(--text-primary); background: var(--bg-hover); }
.nav-btn.active {
  color: var(--accent);
  background: var(--bg-tertiary);
  border: 1px solid var(--border);
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
  background: var(--bg-primary);
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
  color: var(--text-secondary);
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.group-list-panel .panel-header .badge-count {
  font-size: 11px;
  color: var(--text-muted);
  background: var(--bg-tertiary);
  padding: 2px 8px;
  border-radius: 10px;
}
.session-group {
  border-bottom: 1px solid var(--border);
  cursor: pointer;
  transition: background 0.15s;
}
.session-group:hover { background: var(--bg-secondary); }
.session-group.active { background: var(--bg-tertiary); border-left: 3px solid var(--accent); }
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
  color: var(--text-muted);
  background: var(--bg-tertiary);
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
  color: var(--text-muted);
  flex-shrink: 0;
}
.session-group-preview {
  padding: 0 14px 10px 30px;
  font-size: 12px;
  color: var(--text-secondary);
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
  background: var(--bg-primary);
  border-right: 1px solid var(--border);
  overflow-y: auto;
  flex-shrink: 0;
}
.history-group-list .panel-header {
  padding: 12px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 14px;
  font-weight: 600;
  color: var(--text-secondary);
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
  color: var(--text-secondary);
  background: var(--bg-secondary);
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
.project-group .session-item.active { background: var(--bg-tertiary); border-left-color: var(--accent); }
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
  color: var(--text-muted);
  flex-shrink: 0;
}

/* 右侧详情面板 */
.detail-panel {
  flex: 1;
  background: var(--bg-primary);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
.btn {
  padding: 7px 14px;
  border-radius: var(--radius-sm);
  border: 1px solid var(--border);
  background: var(--bg-secondary);
  color: var(--text-primary);
  font-size: 12px;
  cursor: pointer;
  transition: all 0.15s;
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.btn:hover { background: var(--bg-hover); border-color: var(--text-muted); }
.btn-primary { background: var(--accent); border-color: var(--accent); color: #fff; }
.btn-primary:hover { background: var(--accent-hover); border-color: var(--accent-hover); }
.btn-danger { background: transparent; border-color: var(--red); color: var(--red); }
.btn-danger:hover { background: rgba(239,68,68,0.1); border-color: var(--red); }

.empty-state {
  flex:1;
  display:flex;
  align-items:center;
  justify-content:center;
  flex-direction:column;
  gap:12px;
  color: var(--text-muted);
}

/* 设置视图 */
.settings-panel {
  flex: 1;
  overflow-y: auto;
  padding: 32px;
  max-width: 800px;
  margin: 0 auto;
  width: 100%;
}
.settings-panel h1 { font-size: 22px; margin-bottom: 24px; color: var(--accent); }
.settings-section {
  background: var(--bg-secondary);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  padding: 20px;
  margin-bottom: 20px;
}
.settings-section h2 { font-size: 16px; margin-bottom: 16px; color: var(--text-primary); }
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 0;
  border-bottom: 1px solid var(--border);
}
.setting-row:last-child { border-bottom: none; }
.setting-label { font-size: 14px; color: var(--text-secondary); }
.setting-value { font-size: 13px; color: var(--text-muted); }
.status-indicator {
  display: inline-block;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  margin-right: 6px;
}
.status-ok { background: var(--green); }
.status-warn { background: var(--orange); }
.status-error { background: var(--red); }

/* 滚动条 */
::-webkit-scrollbar { width: 8px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: var(--border); border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: var(--text-muted); }
</style>
