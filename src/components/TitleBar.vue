<template>
  <header class="titlebar">
    <span class="brand">
      <img class="brand-icon" src="/icon.png" alt="" />
      CCBuddy
    </span>

    <nav class="nav-links" :aria-label="t('navAria')">
      <button
        v-for="item in navItems"
        :key="item.view"
        class="nav-link"
        :class="{ active: currentView === item.view }"
        @click="emit('navigate', item.view)"
      >
        {{ t(item.key) }}
      </button>
    </nav>

    <div class="spacer"></div>

    <!-- 语言切换（中/EN）：设计稿 .lang-switch，active 态珊瑚 -->
    <div class="lang-switch" role="group" :aria-label="t('langAria')">
      <button
        class="lang-btn"
        :class="{ active: locale === 'zh' }"
        @click="setLocale('zh')"
      >中</button>
      <button
        class="lang-btn"
        :class="{ active: locale === 'en' }"
        @click="setLocale('en')"
      >EN</button>
    </div>

    <!-- 系统存活状态胶囊：alive 由 App.vue 判定（桌面端恒 true；web 端以 SSE 连接状态为准） -->
    <span
      class="alive"
      :class="alive ? 'on' : 'off'"
      :title="alive
        ? `${t('aliveOnTitle')}（${t('pushClockPre')} ${lastPushClock()}）`
        : t('aliveOffTitle')"
    >
      <span class="dot"></span>{{ alive ? t("aliveOn") : t("aliveOff") }}
    </span>
  </header>
</template>

<script setup lang="ts">
import type { View } from "../types";
import type { I18nKey } from "../i18n";
import { locale, setLocale, t } from "../i18n";

const props = defineProps<{
  currentView: View;
  /** 系统是否存活（App.vue 判定：桌面端恒 true；web 端以 SSE 连接状态为准） */
  alive: boolean;
  /** 最近一次收到后端推送的时间戳（Date.now() 毫秒），仅 tooltip 辅助展示 */
  lastPushAt: number;
}>();

const emit = defineEmits<{ navigate: [view: View] }>();

// tooltip 辅助：最近一次推送的钟点时间（静态事实，非时间差，无过期误导）
function lastPushClock(): string {
  return new Date(props.lastPushAt).toLocaleTimeString();
}

// 导航顺序对齐设计稿（用量在前）；view 值与类型定义保持不变
const navItems: { view: View; key: I18nKey }[] = [
  { view: "usage", key: "navUsage" },
  { view: "sessions", key: "navEvents" },
  { view: "history-sessions", key: "navHistory" },
  { view: "settings", key: "navSettings" },
];
</script>

<style scoped>
.titlebar {
  height: 54px;
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border);
  display: flex;
  align-items: center;
  padding: 0 16px;
  gap: 16px;
  flex-shrink: 0;
  user-select: none;
  -webkit-app-region: drag;
}

/* ---- 品牌区 ---- */
.brand {
  font-weight: 700;
  font-size: 17px;
  letter-spacing: 0.2px;
  display: flex;
  align-items: center;
  gap: 9px;
  flex-shrink: 0;
  color: var(--text-1);
}
/* 品牌图标：直接用 Tauri 应用图标（public/icon.png，透明底深色字形）。
   深色标题栏上深色字形不可读，故垫一层近白圆角底当徽标底座。 */
.brand-icon {
  width: 22px;
  height: 22px;
  border-radius: 6px;
  background: var(--text-1);
  display: block;
  flex-shrink: 0;
}

/* ---- 导航（原生文字链接按钮） ---- */
.nav-links {
  display: flex;
  gap: 4px;
  -webkit-app-region: no-drag;
}
.nav-link {
  border: none;
  background: none;
  cursor: pointer;
  color: var(--text-3);
  font-size: 14px;
  font-family: inherit;
  padding: 6px 13px;
  border-radius: var(--radius-sm);
  transition: color 0.15s, background 0.15s;
  position: relative;
}
.nav-link:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.nav-link.active {
  color: var(--text-1);
  font-weight: 600;
}
.nav-link.active::after {
  content: "";
  position: absolute;
  left: 13px;
  right: 13px;
  bottom: 1px;
  height: 2px;
  border-radius: 1px;
  background: var(--accent);
}

.spacer { flex: 1; }

/* ---- 语言切换（中/EN）：active 态珊瑚（设计稿 .lang-switch / .lang-btn） ---- */
.lang-switch {
  display: inline-flex;
  align-items: center;
  gap: 3px;
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  padding: 2px;
  background: var(--bg-elevated);
  user-select: none;
  flex-shrink: 0;
  -webkit-app-region: no-drag;
}
.lang-btn {
  border: none;
  background: none;
  cursor: pointer;
  font-size: 11.5px;
  line-height: 1;
  padding: 4px 9px;
  border-radius: 4px;
  color: var(--text-3);
  font-weight: 600;
  font-family: inherit;
  transition: color 0.15s, background 0.15s;
}
.lang-btn:hover {
  color: var(--text-1);
  background: var(--bg-hover);
}
.lang-btn.active {
  color: var(--accent);
  background: var(--accent-soft);
}

/* ---- 系统存活状态胶囊 ---- */
.alive {
  display: inline-flex;
  align-items: center;
  gap: 8px;
  font-size: 12.5px;
  color: var(--text-3);
  padding: 4px 12px;
  border-radius: 20px;
  border: 1px solid var(--border);
  background: var(--bg-elevated);
  user-select: none;
  cursor: default;
  flex-shrink: 0;
  font-variant-numeric: tabular-nums;
}
.alive .dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
}
/* 运行中：玉翠呼吸点 */
.alive.on {
  color: var(--success);
  border-color: rgba(127, 220, 176, 0.3);
}
.alive.on .dot {
  background: var(--success);
  animation: breathe 2.4s ease-in-out infinite;
}
/* 已断开：纸灰静止点 */
.alive.off .dot {
  background: var(--muted);
  animation: none;
}
@keyframes breathe {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.25; }
}
@media (prefers-reduced-motion: reduce) {
  .alive.on .dot { animation: none; }
}
</style>
