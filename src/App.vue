<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from "vue";
import { invoke } from "@tauri-apps/api/core";

// ---- 类型定义 ----
type SessionStatus =
  | "running"
  | "waiting_confirmation"
  | "waiting_input"
  | "error"
  | "completed"
  | "idle";

interface Message {
  type: "user" | "assistant" | "system";
  role: "user" | "assistant" | "system";
  content: string;
  time: string;
  toolCall?: string;
}

interface Session {
  id: string;
  project: string;
  title: string;
  status: SessionStatus;
  lastActivity: string;
  preview: string;
  unread: boolean;
  messages: Message[];
}

type View = "sessions" | "history-sessions" | "settings";

// ---- 状态 ----
const currentView = ref<View>("sessions");
const selectedSessionId = ref<string | null>(null);
const sessions = ref<Session[]>([]);
const eventsDir = ref("");

let pollTimer: ReturnType<typeof setInterval> | null = null;

// ---- 数据加载 ----
async function loadSessions() {
  try {
    const data = await invoke<Session[]>("get_sessions");
    sessions.value = data.map((s) => ({
      ...s,
      lastActivity: fmtRelative(s.lastActivity),
    }));

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
    eventsDir.value = await invoke<string>("get_events_dir");
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

// ---- 展示辅助 ----
function statusColor(status: SessionStatus): string {
  const map: Record<SessionStatus, string> = {
    running: "var(--green)",
    waiting_confirmation: "var(--orange)",
    waiting_input: "var(--blue)",
    error: "var(--red)",
    completed: "var(--gray)",
    idle: "var(--text-muted)",
  };
  return map[status] ?? "var(--gray)";
}

function statusLabel(status: SessionStatus): string {
  const map: Record<SessionStatus, string> = {
    running: "运行中",
    waiting_confirmation: "需确认",
    waiting_input: "等待输入",
    error: "异常",
    completed: "已完成",
    idle: "空闲",
  };
  return map[status] ?? status;
}

function countByStatus(status: SessionStatus): number {
  return sessions.value.filter((s) => s.status === status).length;
}

/// ISO 时间戳 → 相对时间（"刚刚"/"N分钟前"/"N小时前"/"N天前"）。
function fmtRelative(iso: string): string {
  if (!iso) return "";
  const d = new Date(iso);
  if (isNaN(d.getTime())) return iso;
  const sec = Math.floor((Date.now() - d.getTime()) / 1000);
  if (sec < 60) return "刚刚";
  const min = Math.floor(sec / 60);
  if (min < 60) return `${min}分钟前`;
  const hr = Math.floor(min / 60);
  if (hr < 24) return `${hr}小时前`;
  const day = Math.floor(hr / 24);
  if (day < 7) return `${day}天前`;
  return d.toLocaleDateString("zh-CN");
}

// ---- 交互 ----
function selectSession(session: Session) {
  selectedSessionId.value = session.id;
  session.unread = false;
}

function copyContext(session: Session) {
  const text = session.messages
    .map((m) => `${m.role === "user" ? "用户" : m.role === "assistant" ? "Claude" : "系统"}: ${m.content}`)
    .join("\n");
  navigator.clipboard?.writeText(text).then(
    () => alert("上下文已复制到剪贴板"),
    () => alert("复制失败：剪贴板不可用")
  );
}

function markHandled(session: Session) {
  // TODO: 需后端持久化"已处理"标记，当前仅本地清除未读
  session.unread = false;
  alert(`已标记 "${session.title}" 为已处理`);
}

function locateTerminal(session: Session) {
  // TODO: 定位到终端（可选能力）
  alert(`尝试打开终端并聚焦到 "${session.title}"`);
}

async function installHooks() {
  try {
    const msg = await invoke<string>("install_hooks");
    alert(`✅ ${msg}`);
  } catch (e) {
    alert(`❌ 安装失败：${e}`);
  }
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
  <!-- 顶部标题栏 -->
  <div class="titlebar">
    <span class="logo">CCBuddy</span>
    <span class="badge">Claude Code 会话管理器</span>
    <div class="stats-inline">
      <span class="stat-tag running" @click="currentView = 'sessions'">
        <span class="dot"></span> 运行中 <span class="num">{{ countByStatus('running') }}</span>
      </span>
      <span class="stat-tag confirm" @click="currentView = 'sessions'">
        <span class="dot"></span> 需确认 <span class="num">{{ countByStatus('waiting_confirmation') }}</span>
      </span>
      <span class="stat-tag input" @click="currentView = 'sessions'">
        <span class="dot"></span> 等待输入 <span class="num">{{ countByStatus('waiting_input') }}</span>
      </span>
      <span class="stat-tag error" @click="currentView = 'sessions'">
        <span class="dot"></span> 异常 <span class="num">{{ countByStatus('error') }}</span>
      </span>
      <span class="stat-tag done" @click="currentView = 'sessions'">
        <span class="dot"></span> 已完成 <span class="num">{{ countByStatus('completed') }}</span>
      </span>
    </div>
    <div class="spacer"></div>
    <div class="nav-buttons">
      <button class="nav-btn" :class="{ active: currentView === 'sessions' }" @click="currentView = 'sessions'">事件流</button>
      <button class="nav-btn" :class="{ active: currentView === 'history-sessions' }" @click="currentView = 'history-sessions'">历史会话</button>
      <button class="nav-btn" :class="{ active: currentView === 'settings' }" @click="currentView = 'settings'">设置</button>
    </div>
  </div>

  <div class="main-container">
    <!-- 事件流视图 -->
    <template v-if="currentView === 'sessions'">
      <div class="group-list-panel">
        <div class="panel-header">
          <span>会话事件流</span>
          <span class="badge-count">{{ sortedSessions.length }} 个会话</span>
        </div>
        <div
          v-for="session in sortedSessions"
          :key="session.id"
          class="session-group"
          :class="{ active: selectedSession && selectedSession.id === session.id }"
          @click="selectSession(session)"
        >
          <div class="session-group-header">
            <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
            <span class="project-name">{{ session.project }}</span>
            <span class="session-title">{{ session.title }}</span>
            <span class="time">{{ session.lastActivity }}</span>
            <span v-if="session.unread" class="unread-indicator"></span>
          </div>
          <div class="session-group-preview">{{ session.preview }}</div>
        </div>
        <div v-if="sortedSessions.length === 0" class="empty-state" style="padding: 40px 0;">
          <span style="font-size:48px;">📭</span>
          <span>暂无会话，等待 Claude Code 产生事件</span>
        </div>
      </div>

      <div class="detail-panel">
        <template v-if="selectedSession">
          <div class="detail-header">
            <div class="detail-title">{{ selectedSession.title }}</div>
            <div class="detail-status">
              <span class="status-dot" :style="{ background: statusColor(selectedSession.status) }"></span>
              {{ statusLabel(selectedSession.status) }}
            </div>
            <div class="detail-actions">
              <button class="btn" @click="copyContext(selectedSession)">📋 复制上下文</button>
              <button class="btn" @click="markHandled(selectedSession)">✅ 标记已处理</button>
              <button class="btn btn-danger" @click="locateTerminal(selectedSession)">🖥️ 定位终端</button>
            </div>
          </div>
          <div class="detail-body">
            <div v-for="(msg, idx) in selectedSession.messages" :key="idx" class="message" :class="msg.type">
              <div class="msg-meta">
                <span>{{ msg.role === 'user' ? '👤 用户' : msg.role === 'assistant' ? '🤖 Claude' : '⚠️ 系统' }}</span>
                <span v-if="msg.toolCall" class="tool-call-badge">{{ msg.toolCall }}</span>
                <span style="margin-left:auto;">{{ msg.time }}</span>
              </div>
              <div class="msg-bubble">{{ msg.content }}</div>
            </div>
          </div>
        </template>
        <div v-else class="empty-state">
          <span style="font-size:48px;">🗂️</span>
          <span>选择一个会话查看事件流</span>
        </div>
      </div>
    </template>

    <!-- 历史会话视图 -->
    <template v-else-if="currentView === 'history-sessions'">
      <div class="history-group-list">
        <div class="panel-header">
          <span>历史会话</span>
          <span class="badge-count">{{ sessions.length }} 个会话</span>
        </div>
        <div v-for="project in projectGroups" :key="project.name" class="project-group">
          <div class="project-header">📁 {{ project.name }}</div>
          <div
            v-for="session in project.sessions"
            :key="session.id"
            class="session-item"
            :class="{ active: selectedSession && selectedSession.id === session.id }"
            @click="selectSession(session)"
          >
            <span class="status-dot" :style="{ background: statusColor(session.status) }"></span>
            <span class="session-title">{{ session.title }}</span>
            <span class="session-time">{{ session.lastActivity }}</span>
          </div>
        </div>
      </div>

      <div class="detail-panel">
        <template v-if="selectedSession">
          <div class="detail-header">
            <div class="detail-title">{{ selectedSession.title }}</div>
            <div class="detail-status">
              <span class="status-dot" :style="{ background: statusColor(selectedSession.status) }"></span>
              {{ statusLabel(selectedSession.status) }}
            </div>
            <div class="detail-actions">
              <button class="btn" @click="copyContext(selectedSession)">📋 复制上下文</button>
              <button class="btn" @click="markHandled(selectedSession)">✅ 标记已处理</button>
              <button class="btn btn-danger" @click="locateTerminal(selectedSession)">🖥️ 定位终端</button>
            </div>
          </div>
          <div class="detail-body">
            <div v-for="(msg, idx) in selectedSession.messages" :key="idx" class="message" :class="msg.type">
              <div class="msg-meta">
                <span>{{ msg.role === 'user' ? '👤 用户' : msg.role === 'assistant' ? '🤖 Claude' : '⚠️ 系统' }}</span>
                <span v-if="msg.toolCall" class="tool-call-badge">{{ msg.toolCall }}</span>
                <span style="margin-left:auto;">{{ msg.time }}</span>
              </div>
              <div class="msg-bubble">{{ msg.content }}</div>
            </div>
          </div>
        </template>
        <div v-else class="empty-state">
          <span style="font-size:48px;">🗂️</span>
          <span>选择一个会话查看详情</span>
        </div>
      </div>
    </template>

    <!-- 设置视图 -->
    <div v-else-if="currentView === 'settings'" class="settings-panel">
      <h1>⚙️ CCBuddy 设置</h1>
      <div class="settings-section">
        <h2>Hook 配置</h2>
        <div class="setting-row">
          <span class="setting-label">Hook Logger 状态</span>
          <span><span class="status-indicator status-ok"></span>已安装</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">可执行文件路径</span>
          <span class="setting-value">~/.claude/ccbuddy-hook</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">Claude settings.json 注册</span>
          <span><span class="status-indicator status-warn"></span>未全部注册</span>
        </div>
        <div class="setting-row">
          <button class="btn btn-primary" @click="installHooks">一键安装/更新 Hooks</button>
        </div>
      </div>
      <div class="settings-section">
        <h2>通知</h2>
        <div class="setting-row">
          <span class="setting-label">桌面通知</span>
          <label><input type="checkbox" checked /> 启用</label>
        </div>
        <div class="setting-row">
          <span class="setting-label">通知节流时间</span>
          <input type="number" value="300" style="width:80px; background:var(--bg-tertiary); border:1px solid var(--border); color:var(--text-primary); padding:4px; border-radius:4px;" /> 秒
        </div>
      </div>
      <div class="settings-section">
        <h2>服务器</h2>
        <div class="setting-row">
          <span class="setting-label">监听地址</span>
          <input type="text" value="127.0.0.1:8787" style="width:200px; background:var(--bg-tertiary); border:1px solid var(--border); color:var(--text-primary); padding:4px; border-radius:4px;" />
        </div>
        <div class="setting-row">
          <span class="setting-label">端口冲突处理</span>
          <span class="setting-value">启动时检测，冲突则报错</span>
        </div>
      </div>
      <div class="settings-section">
        <h2>数据目录</h2>
        <div class="setting-row">
          <span class="setting-label">软件数据目录</span>
          <span class="setting-value">~/.ccbuddy/data</span>
        </div>
        <div class="setting-row">
          <span class="setting-label">日志源目录</span>
          <span class="setting-value">{{ eventsDir || '~/.claude/data/events' }}</span>
        </div>
      </div>
    </div>
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
  overflow: hidden;
}

/* 左侧列表面板（事件流视图） */
.group-list-panel {
  width: 420px;
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
.detail-header {
  padding: 16px 24px;
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 12px;
  flex-shrink: 0;
}
.detail-header .detail-title { font-size: 16px; font-weight: 700; flex: 1; }
.detail-header .detail-status {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 12px;
  color: var(--text-secondary);
}
.detail-actions { display: flex; gap: 8px; flex-shrink: 0; }
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

.detail-body {
  flex: 1;
  overflow-y: auto;
  padding: 20px 24px;
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.message { display: flex; flex-direction: column; max-width: 80%; }
.message.user { align-self: flex-end; align-items: flex-end; }
.message.assistant { align-self: flex-start; align-items: flex-start; }
.message.system { align-self: center; width: 100%; }
.msg-bubble {
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 13px;
  line-height: 1.5;
  word-break: break-word;
}
.message.user .msg-bubble { background: var(--bg-tertiary); border: 1px solid var(--border); color: var(--text-primary); }
.message.assistant .msg-bubble { background: var(--bg-secondary); border: 1px solid var(--border); color: var(--text-primary); }
.message.system .msg-bubble {
  background: rgba(239,68,68,0.1);
  border: 1px solid var(--red);
  color: var(--red);
  font-family: var(--font-mono);
  font-size: 12px;
}
.msg-meta {
  font-size: 11px;
  color: var(--text-muted);
  margin-bottom: 4px;
  display: flex;
  align-items: center;
  gap: 6px;
}
.tool-call-badge {
  background: rgba(245,158,11,0.15);
  border: 1px solid var(--orange);
  color: var(--orange);
  padding: 2px 8px;
  border-radius: 4px;
  font-size: 11px;
  font-family: var(--font-mono);
}
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
