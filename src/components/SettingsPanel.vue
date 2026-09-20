<template>
  <div class="settings-panel">
    <h1>{{ t("settingsTitle") }}</h1>

    <n-card :title="t('hookCardTitle')" size="small" class="settings-card">
      <template #header-extra>
        <span class="panel-sub">{{ t("hookCardSub") }}</span>
      </template>
      <div class="setting-row">
        <span class="setting-label">{{ t("hookStatusLabel") }}</span>
        <n-tag :type="hookStatus?.installed ? 'success' : 'error'" size="small" round>
          {{ hookStatus?.installed ? t("installed") : t("notInstalled") }}
        </n-tag>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("hookPath") }}</span>
        <span class="setting-value mono-path">~/.claude/ccbuddy-hook</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("hookRegister") }}</span>
        <n-tag :type="allRegistered ? 'success' : 'warning'" size="small" round>
          {{ registeredCount || "-" }} {{ t("registeredPost") }}
        </n-tag>
      </div>
      <!-- 注册明细：33 个事件逐项状态（纯展示，数据来自 hookStatus.registered） -->
      <div v-if="hookEventsList.length" class="setting-row row-top">
        <span class="setting-label">{{ t("hookDetail") }}</span>
        <div class="hook-events">
          <span
            v-for="ev in hookEventsList"
            :key="ev.name"
            class="hook-ev"
            :class="ev.on ? 'on' : 'off'"
            :title="ev.name + (ev.on ? ' ' + t('hookOnTitle') : ' ' + t('hookOffTitle'))"
          >
            <span class="h-dot"></span>{{ ev.name }}
          </span>
        </div>
      </div>
      <div v-if="hookStatus?.broken" class="setting-row">
        <span class="setting-label">{{ t("hookBroken") }}</span>
        <span class="setting-value warn-text">{{ hookStatus.broken_reason }}</span>
      </div>
      <div class="setting-row row-end row-top">
        <span class="setting-label">{{ t("hookSource") }}</span>
        <span class="setting-value hint-text">
          {{ t("hookSourceHint") }}
        </span>
      </div>
      <div class="set-actions">
        <n-button type="primary" size="small" :loading="installing" @click="installHooks">
          {{ installing ? t("installingText") : t("installBtn") }}
        </n-button>
        <n-popconfirm @positive-click="uninstallHooks">
          <template #trigger>
            <n-button
              size="small"
              :loading="uninstalling"
              :disabled="installing"
            >{{ t("uninstallBtn") }}</n-button>
          </template>
          {{ t("uninstallConfirm") }}
        </n-popconfirm>
      </div>
    </n-card>

    <n-card :title="t('serverCardTitle')" size="small" class="settings-card">
      <template #header-extra>
        <span class="panel-sub">{{ t("serverCardSub") }}</span>
      </template>
      <div class="setting-row">
        <span class="setting-label">{{ t("serverUrlLabel") }}</span>
        <a :href="serverUrl" target="_blank" rel="noopener" class="link mono-path">{{ serverUrl }}</a>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("portLabel") }}</span>
        <span class="setting-value">{{ t("portValue") }}</span>
      </div>
    </n-card>

    <!-- 访问密码：只有 Web 版（ccbuddy-server）有登录密码，桌面端不显示 -->
    <n-card v-if="!isTauri" :title="t('pwdCardTitle')" size="small" class="settings-card">
      <template #header-extra>
        <span class="panel-sub">{{ t("pwdCardSub") }}</span>
      </template>
      <div class="setting-row">
        <span class="setting-label">{{ t("pwdOld") }}</span>
        <n-input
          v-model:value="oldPwd"
          type="password"
          size="small"
          show-password-on="click"
          class="pwd-input"
        />
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("pwdNew") }}</span>
        <n-input
          v-model:value="newPwd"
          type="password"
          size="small"
          :placeholder="t('pwdPh')"
          show-password-on="click"
          class="pwd-input"
        />
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("pwdConfirm") }}</span>
        <n-input
          v-model:value="newPwd2"
          type="password"
          size="small"
          show-password-on="click"
          class="pwd-input"
          @keyup.enter="changePwd"
        />
      </div>
      <div class="set-actions">
        <n-button type="primary" size="small" :loading="pwdSaving" @click="changePwd">
          {{ t("pwdChangeBtn") }}
        </n-button>
        <n-button size="small" @click="logout">{{ t("logoutBtn") }}</n-button>
      </div>
    </n-card>

    <n-card :title="t('dirCardTitle')" size="small" class="settings-card">
      <template #header-extra>
        <span class="panel-sub">{{ t("dirCardSub") }}</span>
      </template>
      <div class="setting-row">
        <span class="setting-label">{{ t("dirClaude") }}</span>
        <n-input-group style="width: 420px">
          <n-input
            v-model:value="claudeDir"
            size="small"
            :placeholder="t('claudeDirPh')"
            clearable
            @keyup.enter="saveClaudeDir"
          />
          <n-button size="small" :loading="claudeDirSaving" @click="saveClaudeDir">
            {{ claudeDirSaved ? t("savedBtn") : t("saveBtn") }}
          </n-button>
        </n-input-group>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("dirData") }}</span>
        <span class="setting-value mono-path">~/.ccbuddy/data</span>
      </div>
    </n-card>

    <n-card :title="t('aboutCardTitle')" size="small" class="settings-card">
      <template #header-extra>
        <span class="panel-sub">{{ t("aboutCardSub") }}</span>
      </template>
      <div class="setting-row">
        <span class="setting-label">{{ t("aboutVersion") }}</span>
        <span class="setting-value">v{{ APP_VERSION }}</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("aboutRepo") }}</span>
        <a :href="REPO_URL" target="_blank" rel="noopener" class="link">{{ REPO_URL }}</a>
      </div>
      <div class="setting-row">
        <span class="setting-label">{{ t("aboutSite") }}</span>
        <a :href="SITE_URL" target="_blank" rel="noopener" class="link">{{ SITE_URL }}</a>
      </div>
    </n-card>
  </div>
</template>

<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { useNotification } from "naive-ui";
import {
  installHooks as callInstallHooks,
  uninstallHooks as callUninstallHooks,
  getHookStatus,
  getConfig,
  setConfig,
  type HookStatus,
} from "../api";
import { t } from "../i18n";
import { changePassword, logout } from "../auth";

// ---- 关于卡片：版本与链接 ----
// 版本取自 package.json（构建期注入）；官网为占位地址，待正式发布后替换。
const APP_VERSION = __APP_VERSION__;
const REPO_URL = "https://github.com/xutopia77/ccbuddy";
const SITE_URL = "https://xutopia.top/project/ccbuddy";

const notification = useNotification();

// ---- hook 状态 ----
const hookStatus = ref<HookStatus | null>(null);
const installing = ref(false);
const uninstalling = ref(false);

async function refreshHookStatus() {
  try {
    hookStatus.value = await getHookStatus();
  } catch (e) {
    console.error("获取 hook 状态失败", e);
  }
}

async function installHooks() {
  if (installing.value) return;
  installing.value = true;
  try {
    const msg = await callInstallHooks();
    notification.success({ title: t("toastInstallDone"), content: msg, duration: 4000 });
    await refreshHookStatus();
  } catch (e) {
    // 后端错误信息含手动放置步骤提示
    const detail = e instanceof Error ? e.message : String(e);
    notification.error({
      title: t("toastInstallFail"),
      content: detail,
      duration: 0, // 含手动处理步骤，不自动消失
    });
  } finally {
    installing.value = false;
  }
}

async function uninstallHooks() {
  if (uninstalling.value) return;
  uninstalling.value = true;
  try {
    const msg = await callUninstallHooks();
    notification.success({ title: t("toastUninstallDone"), content: msg, duration: 4000 });
    await refreshHookStatus();
  } catch (e) {
    const detail = e instanceof Error ? e.message : String(e);
    notification.error({ title: t("toastUninstallFail"), content: detail, duration: 0 });
  } finally {
    uninstalling.value = false;
  }
}

// 已注册的事件数 / 总事件数
const registeredCount = computed(() => {
  if (!hookStatus.value) return "";
  const vals = Object.values(hookStatus.value.registered);
  const ok = vals.filter(Boolean).length;
  return `${ok}/${vals.length}`;
});
const allRegistered = computed(() => {
  if (!hookStatus.value) return false;
  const vals = Object.values(hookStatus.value.registered);
  return vals.length > 0 && vals.every(Boolean);
});

// 注册明细列表（纯展示）：事件名 → 是否已注册
const hookEventsList = computed(() => {
  if (!hookStatus.value) return [];
  return Object.entries(hookStatus.value.registered).map(([name, on]) => ({ name, on }));
});

// ---- Claude 数据目录（后端配置，~/.ccbuddy/config.json）----
const claudeDir = ref("");
const claudeDirSaving = ref(false);
const claudeDirSaved = ref(false);

async function loadConfig() {
  try {
    const cfg = await getConfig();
    claudeDir.value = cfg.claude_dir;
  } catch {
    /* 用默认值 */
  }
}

async function saveClaudeDir() {
  if (claudeDirSaving.value) return;
  claudeDirSaving.value = true;
  try {
    const cfg = await setConfig({ claude_dir: claudeDir.value });
    claudeDir.value = cfg.claude_dir;
    claudeDirSaved.value = true;
    setTimeout(() => (claudeDirSaved.value = false), 1500);
    // 目录变化会影响 hook 安装位置与注册状态
    await refreshHookStatus();
  } catch (e) {
    const detail = e instanceof Error ? e.message : String(e);
    notification.error({ title: t("toastSaveFail"), content: detail, duration: 5000 });
  } finally {
    claudeDirSaving.value = false;
  }
}

// ---- 访问密码（Web 版登录密码）----
const oldPwd = ref("");
const newPwd = ref("");
const newPwd2 = ref("");
const pwdSaving = ref(false);

async function changePwd() {
  if (pwdSaving.value) return;
  if (newPwd.value !== newPwd2.value) {
    notification.error({ title: t("toastSaveFail"), content: t("pwdMismatch"), duration: 4000 });
    return;
  }
  pwdSaving.value = true;
  try {
    // 成功即改完：服务端撤销了全部会话，auth 内部把登录态归零，界面自动回登录页。
    // 提示由 n-notification-provider 渲染（它在登录门控之外，不会随子树卸载消失）
    await changePassword(oldPwd.value, newPwd.value);
    notification.success({ title: t("pwdChanged"), duration: 5000 });
  } catch (e) {
    // 后端错误信息可直接展示（原密码错误 / 至少 6 位 / 环境变量锁定）
    const detail = e instanceof Error ? e.message : String(e);
    notification.error({ title: t("toastSaveFail"), content: detail, duration: 5000 });
  } finally {
    pwdSaving.value = false;
  }
}

// ---- 服务器地址 ----
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
// Web 版（ccbuddy-server）的访问地址就是当前页面；桌面版提供本地服务的默认地址
const serverUrl = isTauri ? "http://127.0.0.1:18787" : window.location.origin;

onMounted(() => {
  refreshHookStatus();
  loadConfig();
});
</script>

<style scoped>
.settings-panel {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 24px;
  max-width: 760px;
  margin: 0 auto; /* 居中，避免面板靠左、滚动条悬在界面中间 */
  width: 100%;
}
.settings-panel h1 {
  font-size: 18px;
  margin-bottom: 16px;
  color: var(--text-1);
}
.settings-card {
  margin-bottom: 16px;
}
.setting-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 9px 0;
  border-bottom: 1px solid var(--border);
  flex-wrap: wrap;
}
.setting-row:last-child {
  border-bottom: none;
}
/* .set-actions 自带分隔上边框，其前一行不再画下边框（避免双重线） */
.setting-row.row-end {
  border-bottom: none;
}
/* 顶部对齐行：多行说明文字 / 明细网格 */
.setting-row.row-top {
  align-items: flex-start;
}
.setting-label {
  color: var(--text-2);
  font-size: 13.5px;
  flex-shrink: 0;
}
.setting-value {
  color: var(--text-1);
  font-size: 13px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.mono-path {
  font-family: var(--font-mono);
  font-size: 12px;
  word-break: break-all;
}
.hint-text {
  max-width: 480px;
  color: var(--text-3);
  font-size: 12px;
  line-height: 1.6;
  text-align: right;
}
.warn-text {
  max-width: 480px;
  color: var(--warning);
  font-size: 12px;
  line-height: 1.6;
}
.hint-text a,
.link {
  color: var(--accent);
  text-decoration: none;
}
/* 卡片头部右侧副标题（设计稿 panel-sub） */
.panel-sub {
  font-size: 12.5px;
  color: var(--text-3);
}
/* 密码输入框（与 claudeDir 输入组同宽，视觉对齐） */
.pwd-input {
  width: 240px;
}
/* hook 事件注册明细网格（设计稿 .hook-events） */
.hook-events {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
  gap: 5px 14px;
  width: 100%;
  margin-top: 4px;
}
.hook-ev {
  display: flex;
  align-items: center;
  gap: 6px;
  font-size: 11.5px;
  color: var(--text-3);
  font-family: var(--font-mono);
}
.hook-ev .h-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  flex-shrink: 0;
}
.hook-ev.on {
  color: var(--text-2);
}
.hook-ev.on .h-dot {
  background: var(--success); /* 设计稿 --st-run（玉翠） */
}
.hook-ev.off .h-dot {
  background: var(--danger); /* 设计稿 --st-error（猩红） */
}
/* 安装/卸载操作区（设计稿 .set-actions：上边框分隔 + 顶部留白） */
.set-actions {
  display: flex;
  align-items: center;
  gap: 14px;
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--border);
  flex-wrap: wrap;
}
</style>
