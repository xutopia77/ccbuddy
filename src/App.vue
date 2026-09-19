<template>
  <n-config-provider :theme="darkTheme" :theme-overrides="themeOverrides" :locale="locale === 'zh' ? zhCN : enUS" style="height:100%; display:flex; flex-direction:column;">
    <n-notification-provider placement="bottom-right">
      <n-message-provider placement="bottom">
        <!-- 登录门控：桌面端 authed 恒为 true（无登录概念），Web 版未登录只显示登录页。
             authed === null 是「尚未探测」——什么都不渲染（body 已是深色底，无闪白） -->
        <template v-if="authed === true">
        <TitleBar :current-view="currentView" :alive="alive" :last-push-at="lastPushAt" @navigate="currentView = $event" />

        <div
          class="main-container"
          :class="{ 'duo-view': currentView === 'sessions' || currentView === 'history-sessions' }"
        >
          <!-- 事件流视图（设计稿 .duo：左列表 card + 右详情 card，各自独立滚动） -->
          <template v-if="currentView === 'sessions'">
            <aside class="duo-side" :style="{ width: eventsListWidth + 'px' }">
              <EventList
                :sessions="sortedSessions"
                :selected-id="selectedEventId"
                class="duo-side-fill"
                @select="selectSession"
              />
            </aside>
            <SplitDivider v-model="eventsListWidth" storage-key="ccbuddy:splitter-events" :default-width="320" />
            <SessionDetail
              class="duo-main"
              :session="selectedSession"
              :empty-text="t('eventsEmptyText')"
            />
          </template>

          <!-- 历史会话视图（设计稿 .duo.triple 三栏：左列表 card + 中对话流 card + 右用户输入 card） -->
          <template v-else-if="currentView === 'history-sessions'">
            <aside class="duo-side" :style="{ width: historyListWidth + 'px' }">
              <SessionList
                :groups="projectGroups"
                :selected-id="selectedHistoryId"
                :refreshing="historyRefreshing"
                class="duo-side-fill"
                @select="selectSession"
                @refresh="refreshHistory"
              />
            </aside>
            <SplitDivider v-model="historyListWidth" storage-key="ccbuddy:splitter-sessions" :default-width="420" />
            <SessionDetail
              ref="historyDetailRef"
              class="duo-main"
              :session="selectedSession"
              mode="chat"
              :empty-text="t('historyEmptyText')"
            />
            <!-- 右栏：用户输入快速定位（设计稿三栏独立网格第三列，248px） -->
            <aside
              v-if="historyDetailRef?.userInputs?.length"
              class="user-inputs-panel"
            >
              <div class="user-inputs-header">{{ t("userInputsHeader") }} {{ historyDetailRef.userInputs.length }}</div>
              <div class="user-inputs-scroll">
                <div
                  v-for="({ msg, idx }, i) in historyDetailRef.userInputs"
                  :key="idx"
                  class="user-input-item"
                  :class="{ active: historyDetailRef.activeUserIdx === idx }"
                  @click="historyDetailRef?.scrollToUser(idx)"
                >
                  <span class="user-input-idx">{{ i + 1 }}</span>
                  <span class="user-input-text">{{ historyDetailRef.summarize(msg, 40) }}</span>
                </div>
              </div>
            </aside>
          </template>

          <!-- 用量视图（API 请求 token 计量，全宽单栏表格，不插 SplitDivider） -->
          <UsagePanel v-else-if="currentView === 'usage'" />

          <!-- 设置视图 -->
          <SettingsPanel v-else-if="currentView === 'settings'" />
        </div>
        </template>

        <LoginView v-else-if="authed === false" />
      </n-message-provider>
    </n-notification-provider>
  </n-config-provider>
</template>

<script setup lang="ts">
import { ref, computed, onUnmounted, watch } from "vue";
import type { Session, SessionStatus, View } from "./types";
import { ack } from "./ack";
import { getEvents, getSessions, getSessionDetail, getEventDetail } from "./api";
import { closeSse, onEvent, onSseConnectionChange, isTauri as isTauriApp, type UnlistenFn } from "./ipc";
import { authed, startAuth } from "./auth";
import { darkTheme, zhCN, enUS, createDiscreteApi, type GlobalThemeOverrides } from "naive-ui";
import { t, locale } from "./i18n";
import TitleBar from "./components/TitleBar.vue";
import EventList from "./components/EventList.vue";
import SessionList from "./components/SessionList.vue";
import SessionDetail from "./components/SessionDetail.vue";
import SettingsPanel from "./components/SettingsPanel.vue";
import SplitDivider from "./components/SplitDivider.vue";
import UsagePanel from "./components/UsagePanel.vue";
import LoginView from "./components/LoginView.vue";

// Naive UI 暗色主题，主题量与 src/styles/tokens.css 保持同值（唯一样式源）
// 项目仅暗色模式（tokens.css 只有 :root 一套暗色值，无主题切换机制），故只有 dark 一份 overrides
const themeOverrides: GlobalThemeOverrides = {
  common: {
    // ---- 品牌色（--accent / --accent-hover / --accent-dim）----
    primaryColor: "#ffa063",
    primaryColorHover: "#ffb387",
    primaryColorPressed: "#c96a44",
    primaryColorSuppl: "#ffb387",

    // ---- 表面（--bg-base / --bg-surface / --bg-elevated / --bg-code）----
    bodyColor: "#090b0d",
    cardColor: "#151b23",
    modalColor: "#1b232e",
    popoverColor: "#1b232e",
    tableColor: "#151b23",
    inputColor: "#0b0e12",
    actionColor: "#1b232e",

    // ---- 边框（--border / --border-strong）----
    borderColor: "rgba(255, 255, 255, 0.14)",
    dividerColor: "rgba(255, 255, 255, 0.07)",

    // ---- 文本（--text-1 / --text-2 / --text-3）----
    textColorBase: "#eff2f6",
    textColor1: "#eff2f6",
    textColor2: "#bfcad8",
    textColor3: "#9dabbf",

    // ---- 语义色（--success / --warning / --danger / --info）----
    successColor: "#7fdcb0",
    successColorHover: "#97e4c0",
    successColorPressed: "#63c49a",
    // warning 与 primary 同值：Midnight Coral 中“需你出手”即品牌色
    warningColor: "#ffa063",
    warningColorHover: "#ffb387",
    warningColorPressed: "#c96a44",
    errorColor: "#ff6355",
    errorColorHover: "#ff7f73",
    errorColorPressed: "#d94c40",
    infoColor: "#8fb7e8",
    infoColorHover: "#a7c7ee",
    infoColorPressed: "#6f9dd4",

    // ---- 圆角（设计稿 --r-inset 8px）----
    borderRadius: "8px",
    borderRadiusSmall: "6px",

    fontWeightStrong: "700",

    // ---- 字体（--font-sans / --font-mono）----
    fontFamily:
      "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
    fontFamilyMono:
      "'SF Mono', 'JetBrains Mono', 'Fira Code', Consolas, monospace",
  },
  // 卡片顶栏标题与正文同用 --text-1，避免 Naive 默认灰阶
  Card: {
    titleTextColor: "#eff2f6",
    borderColor: "rgba(255, 255, 255, 0.07)",
    borderRadius: "12px",
  },
  Tag: {
    borderRadius: "16px",
  },
  Button: {
    fontWeight: "600",
  },
};

// ---- 状态 ----
// 默认视图为用量页（主页）；事件流/历史会话仍可从顶栏切换
const currentView = ref<View>("usage");
// 事件流视图选中会话（hook 日志，实时）
const selectedEventId = ref<string | null>(null);
// 历史会话视图选中会话（原生 transcript）
const selectedHistoryId = ref<string | null>(null);
// 事件流会话（hook 日志，实时）
const sessions = ref<Session[]>([]);
// 历史会话（Claude Code 原生 transcript），与事件流分开加载
const historySessions = ref<Session[]>([]);
// 事件流选中会话的完整详情（懒加载：列表的 messages 为空，点开后单独拉取）
const eventDetail = ref<Session | null>(null);
// 历史会话选中会话的完整详情（懒加载）
const historyDetail = ref<Session | null>(null);
// 两栏分栏宽度（px）：由 SplitDivider 拖动/持久化，仅作用于列表面板（详情 flex:1 占余下宽度）
const eventsListWidth = ref(320);
const historyListWidth = ref(420);
// ---- 系统存活状态（顶栏存活指示）----
// 桌面端 Tauri 为进程内 invoke 通信，无断连概念，恒为运行中；
// web 端以 SSE 连接状态为权威信号（EventSource 断线自动重连，重连成功恢复运行中）。
const alive = ref(isTauriApp);
// web 端最近一次收到推送的时间（仅用于 tooltip 辅助文案）
const lastPushAt = ref(Date.now());

// 历史视图详情组件实例：读 userInputs/activeUserIdx/summarize、调 scrollToUser（右栏 card 由本组件渲染）
const historyDetailRef = ref<InstanceType<typeof SessionDetail> | null>(null);

// 独立 message（App.vue 是 provider 宿主，自身不在 provider 内，用 discrete API 弹提示）
const { message: discreteMessage } = createDiscreteApi(["message"], {
  configProviderProps: { theme: darkTheme, themeOverrides },
});

// ---- 历史会话手动刷新（设计稿 .refresh-btn：历史不自动刷新，按钮手动重载）----
const historyRefreshing = ref(false);
async function refreshHistory() {
  if (historyRefreshing.value) return;
  historyRefreshing.value = true;
  try {
    // 重载列表；详情重新拉取当前选中会话（先置空触发 SessionDetail 的 id watch，
    // 重载后展开态归零——对齐设计稿"清空展开态"语义）
    await loadHistorySessions();
    const keepId = selectedHistoryId.value;
    historyDetail.value = null;
    if (keepId) {
      historyDetail.value = await getSessionDetail(keepId);
    }
    discreteMessage.success(t("refreshDone"));
  } catch (e) {
    console.error("刷新历史会话失败", e);
  } finally {
    historyRefreshing.value = false;
  }
}

let unlistenEvents: UnlistenFn | null = null;
let unlistenUsagePing: UnlistenFn | null = null;
let unlistenSseConn: UnlistenFn | null = null;
let detailLoading = false;

// ---- 数据加载 ----
async function loadEvents() {
  try {
    // lastActivity 保留原始 ISO 时间戳，由各组件按需格式化
    const data = await getEvents();
    applyEvents(data);
  } catch (e) {
    console.error("加载事件流失败", e);
  }
}

/** 应用事件流数据（轮询与 SSE 推送共用）。 */
function applyEvents(data: Session[]) {
  sessions.value = data;
  lastPushAt.value = Date.now();

  // 保持选中状态：若当前选中会话已消失则清除，否则若未选中则选最紧急的
  if (selectedEventId.value && !sessions.value.find((s) => s.id === selectedEventId.value)) {
    selectedEventId.value = null;
  }
  if (!selectedEventId.value && sessions.value.length > 0) {
    const firstUrgent =
      sessions.value.find((s) => s.status === "waiting_confirmation") ??
      sessions.value.find((s) => s.status === "error");
    selectedEventId.value = firstUrgent ? firstUrgent.id : sessions.value[0].id;
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
  const isHistory = currentView.value === "history-sessions";
  try {
    // 数据源分开：事件流视图读 hook 日志（最新50条），历史视图读原生 transcript（全量）
    if (isHistory) {
      historyDetail.value = await getSessionDetail(id);
    } else {
      eventDetail.value = await getEventDetail(id);
    }
  } catch (e) {
    console.error("加载会话详情失败", e);
    // 详情加载失败时回退到当前视图列表里的概要数据（messages 为空）
    const pool = isHistory ? historySessions.value : sessions.value;
    const fallback = pool.find((s) => s.id === id) ?? null;
    if (isHistory) {
      historyDetail.value = fallback;
    } else {
      eventDetail.value = fallback;
    }
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
  // 依据当前视图返回各自的选中项：详情优先、回退列表概要
  if (currentView.value === "history-sessions") {
    if (historyDetail.value && historyDetail.value.id === selectedHistoryId.value) {
      return historyDetail.value;
    }
    return historySessions.value.find((s) => s.id === selectedHistoryId.value) ?? null;
  }
  if (eventDetail.value && eventDetail.value.id === selectedEventId.value) {
    return eventDetail.value;
  }
  return sessions.value.find((s) => s.id === selectedEventId.value) ?? null;
});

const projectGroups = computed(() => {
  const groups: Record<string, Session[]> = {};
  historySessions.value.forEach((s) => {
    if (!groups[s.project]) groups[s.project] = [];
    groups[s.project].push(s);
  });
  return Object.keys(groups).map((name) => ({ name, sessions: groups[name] }));
});

// ---- 展示辅助 ----

// ---- 交互 ----
function selectSession(session: Session) {
  // 按当前视图写入对应视图的选中状态
  if (currentView.value === "history-sessions") {
    selectedHistoryId.value = session.id;
    historyDetail.value = null;
  } else {
    selectedEventId.value = session.id;
    eventDetail.value = null;
    // 点开即视为已读：列表不再对该状态高亮提醒（后端 unread 是状态派生量，
    // 下次推送会重新变 true，故用 ack 记录「看到哪一条」持久化下来）
    session.unread = false;
    ack(session);
  }
  // 懒加载：点开时才解析该会话的完整 jsonl
  loadDetail(session.id);
}

// 切换视图：清空选中与详情；每次进入历史视图都重新获取列表（不参与自动刷新）
watch(currentView, (view) => {
  selectedEventId.value = null;
  selectedHistoryId.value = null;
  eventDetail.value = null;
  historyDetail.value = null;
  if (view === "history-sessions") {
    loadHistorySessions();
  }
});

// ---- 生命周期 ----
// 订阅只在**已登录**时建立。桌面端 authed 恒为 true，行为与改造前一致；
// Web 版未登录就订阅会建出一条被 401 打死、且按规范不重试的 SSE 连接，
// 登录后所有推送都会挂在那条死连接上（见 ipc.ts closeSse）。
watch(
  authed,
  (ok) => {
    if (ok === null) return; // 尚未探测：不动
    if (ok) {
      // 推送优先：后端 watcher 检测到事件流变化时主动推（SSE / Tauri event），零轮询
      unlistenEvents = onEvent<Session[]>("events_changed", applyEvents);
      // usage_changed 由 UsagePanel 消费数据，App 只借同一推送记录最近推送时间（tooltip 辅助）
      unlistenUsagePing = onEvent("usage_changed", () => (lastPushAt.value = Date.now()));
      if (isTauriApp) {
        // 桌面端 Tauri event 无首帧，登录态确定时拉一次
        loadEvents();
      } else {
        // web 端存活判定：SSE 连接状态为权威信号（注册时立即按当前 readyState 回调一次）。
        // SSE 首帧即全量列表（服务端连接建立时推一次），无需主动 loadEvents
        unlistenSseConn = onSseConnectionChange((v) => (alive.value = v));
      }
    } else {
      if (!isTauriApp) {
        // 真的关掉连接（不只是退订），否则重新登录后仍挂在死连接上
        closeSse();
        alive.value = false;
      }
      unlistenEvents?.();
      unlistenUsagePing?.();
      unlistenSseConn?.();
      unlistenEvents = null;
      unlistenUsagePing = null;
      unlistenSseConn = null;
    }
  },
  { immediate: true }
);

// 登录态维护：首屏探测 + 全局 401 回调 + 可见时心跳（桌面端内部直接跳过）
startAuth();

onUnmounted(() => {
  unlistenEvents?.();
  unlistenUsagePing?.();
  unlistenSseConn?.();
});
</script>

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

/* 左右列表面板样式（.group-list-panel / .history-group-list / .sitem / .proj-label 等）
   已随第三批视觉改造移入 EventList.vue / SessionList.vue 各自的 scoped style，
   宽度由 App.vue 内联 style（SplitDivider 控制）注入，此处不再保留样式副本 */

/* ===== 双 card 布局（设计稿 .duo）：左列表 card + 右详情 card，各自独立滚动 ===== */
/* 左列表外框 card：宽度由内联 style（SplitDivider）控制，card 内列表组件独立滚动 */
.duo-side {
  display: flex;
  flex-direction: column;
  min-height: 0;
  min-width: 0;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  overflow: hidden;
  box-shadow: var(--shadow);
}
.duo-side-fill {
  flex: 1;
  min-height: 0;
}

/* 双 card 布局的呼吸间隙：对齐设计稿 .duo gap:18px——卡与内容区边缘留白、
   卡间为纯空隙（无分隔线边框，SplitDivider 占位拉宽为空隙本身，拖拽功能保留） */
.main-container.duo-view {
  padding: 14px;
  gap: 0;
}
.main-container.duo-view > .split-divider {
  /* 分隔线即卡间空隙：18px 占位（设计稿 .duo gap），透明不可见但可拖拽 */
  align-self: stretch;
  border-left: none;
}

/* 窄窗口（<1000px）：双 card 竖向堆叠，取消拖拽分隔线，各 card 限高自滚 */
@media (max-width: 1000px) {
  .main-container.duo-view {
    flex-direction: column;
    overflow-y: auto;
  }
  .main-container.duo-view > .split-divider {
    display: none;
  }
  .main-container.duo-view > .duo-side {
    width: auto !important; /* 覆盖 SplitDivider 注入的内联宽度 */
    flex-shrink: 0;
    max-height: 40vh;
  }
  .main-container.duo-view > .duo-main {
    flex-shrink: 0;
    min-height: 60vh;
  }
  /* 右栏用户输入 card：窄屏收窄且限高，不再独占一列 */
  .main-container.duo-view > .user-inputs-panel {
    width: auto;
    margin-left: 0;
    max-height: 30vh;
  }
}

/* 右详情外框 card：SessionDetail 根节点 .detail-panel 由其 scoped 提供 flex:1，
   此处只补 card 外观；背景改由 card 自身提供 */
.duo-main.detail-panel {
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow);
}

/* ===== 右栏：用户输入快速定位 card（设计稿 .user-inputs-panel，三栏独立第三列） ===== */
.user-inputs-panel {
  width: 248px;
  flex-shrink: 0;
  margin: 0 0 0 18px; /* 卡间空隙（设计稿 .duo gap:18px） */
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-card);
  box-shadow: var(--shadow);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  min-height: 0;
}
.user-inputs-header {
  padding: 11px 16px;
  border-bottom: 1px solid var(--border);
  font-size: 13px;
  font-weight: 700;
  color: var(--text-2);
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
.user-inputs-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 6px 8px 12px;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.user-input-item {
  display: flex;
  align-items: baseline;
  gap: 8px;
  padding: 6px 9px;
  border-radius: var(--radius-inset);
  font-size: 12.5px;
  color: var(--text-2);
  line-height: 1.45;
  cursor: pointer;
  transition: background 0.12s, color 0.12s;
}
.user-input-item:hover { background: var(--bg-hover); color: var(--text-1); }
.user-input-item.active { background: var(--warning-soft); color: var(--warning); }
.user-input-idx {
  flex-shrink: 0;
  min-width: 16px;
  text-align: right;
  font-family: var(--font-mono);
  font-size: 10.5px;
  font-weight: 700;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}
.user-input-item.active .user-input-idx { color: var(--warning); }
.user-input-text {
  overflow: hidden;
  text-overflow: ellipsis;
  display: -webkit-box;
  -webkit-line-clamp: 2;
  -webkit-box-orient: vertical;
  min-width: 0;
}

/* 两栏分隔线（可拖拽调整列表宽度）：双 card 布局中为卡间空隙本身——
   设计稿 .duo gap 18px 的可拖版本，默认透明，hover/拖动时才显珊瑚提示 */
.split-divider {
  width: 18px;
  flex-shrink: 0;
  cursor: col-resize;
  background: transparent;
  transition: background 0.15s;
}
.split-divider:hover,
.split-divider.dragging {
  background: var(--accent-soft);
}

/* 右侧详情面板 */
.detail-panel {
  flex: 1;
  background: var(--bg-base);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
/* 滚动条（对齐设计稿：细 thumb + 2px 透明边框留出轨道间隙） */
::-webkit-scrollbar { width: 9px; height: 9px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb {
  background: rgba(255, 255, 255, 0.09);
  border-radius: 5px;
  border: 2px solid transparent;
  background-clip: content-box;
}
::-webkit-scrollbar-thumb:hover { background-color: rgba(255, 255, 255, 0.2); }
</style>
