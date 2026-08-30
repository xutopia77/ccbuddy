<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { useNotification } from "naive-ui";
import { installHooks as callInstallHooks, getHookStatus, getConfig, setConfig, type HookStatus } from "../api";
import type { PollConfig } from "../types";

defineProps<{ eventsDir: string }>();

const notification = useNotification();

// ---- hook 状态 ----
const hookStatus = ref<HookStatus | null>(null);
const installing = ref(false);

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
    notification.success({ title: "Hook 安装完成", content: msg, duration: 4000 });
    await refreshHookStatus();
  } catch (e) {
    // 后端错误信息含下载地址与离线手动处理步骤
    const detail = e instanceof Error ? e.message : String(e);
    notification.error({
      title: "安装失败",
      content: detail,
      duration: 0, // 含手动处理步骤，不自动消失
    });
  } finally {
    installing.value = false;
  }
}

// 已注册的事件数 / 总事件数
const registeredCount = computed(() => {
  if (!hookStatus.value) return "";
  const vals = Object.values(hookStatus.value.registered);
  const ok = vals.filter(Boolean).length;
  return `${ok}/${vals.length}`;
});
const allRegistered = computed(() => registeredCount.value === "8/8");

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
    notification.error({ title: "保存失败", content: detail, duration: 5000 });
  } finally {
    claudeDirSaving.value = false;
  }
}

// ---- 轮询设置（纯前端量，localStorage 持久化）----
const pollEnabled = ref(true);
const pollInterval = ref(2);

function loadPollConfig() {
  try {
    const raw = localStorage.getItem("ccbuddy.poll");
    if (raw) {
      const cfg = JSON.parse(raw) as PollConfig;
      pollEnabled.value = cfg.enabled;
      pollInterval.value = cfg.intervalSec;
    }
  } catch {
    /* 损坏则用默认 */
  }
}

function savePollConfig() {
  const cfg: PollConfig = {
    enabled: pollEnabled.value,
    intervalSec: Math.max(1, Number(pollInterval.value) || 2),
  };
  pollInterval.value = cfg.intervalSec;
  localStorage.setItem("ccbuddy.poll", JSON.stringify(cfg));
  // 通知 App.vue 重新应用轮询（重新挂载 onMounted 逻辑太重，直接 dispatch 事件）
  window.dispatchEvent(new CustomEvent("ccbuddy:poll-config-changed"));
}

// ---- 服务器地址 ----
const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
// Web 版（ccbuddy-server）的访问地址就是当前页面；桌面版提供本地服务的默认地址
const serverUrl = isTauri ? "http://127.0.0.1:8787" : window.location.origin;

onMounted(() => {
  refreshHookStatus();
  loadPollConfig();
  loadConfig();
});
</script>

<template>
  <div class="settings-panel">
    <h1>⚙️ CCBuddy 设置</h1>

    <n-card title="Hook 配置" size="small" class="settings-card">
      <div class="setting-row">
        <span class="setting-label">Hook Logger 状态</span>
        <n-tag :type="hookStatus?.installed ? 'success' : 'error'" size="small" round>
          {{ hookStatus?.installed ? "已安装" : "未安装" }}
        </n-tag>
      </div>
      <div class="setting-row">
        <span class="setting-label">可执行文件路径</span>
        <span class="setting-value">~/.claude/ccbuddy-hook</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">Claude settings.json 注册</span>
        <n-tag :type="allRegistered ? 'success' : 'warning'" size="small" round>
          {{ registeredCount || "-" }} 个事件已注册
        </n-tag>
      </div>
      <div class="setting-row">
        <span class="setting-label">hook 安装来源</span>
        <span class="setting-value hint-text">
          安装时优先使用本地 hook（安装包内置 / 便携包同目录 / ~/.ccbuddy/bin），
          本地没有则自动从 GitHub Release 下载；离线环境可
          <a :href="hookStatus?.downloadUrl" target="_blank" rel="noopener">手动下载</a>
          后放入 ~/.ccbuddy/bin/
        </span>
      </div>
      <div class="setting-row">
        <n-button type="primary" size="small" :loading="installing" @click="installHooks">
          {{ installing ? "正在安装（可能正在下载 hook）..." : "一键安装 / 更新 Hooks" }}
        </n-button>
      </div>
    </n-card>

    <n-card title="事件刷新" size="small" class="settings-card">
      <div class="setting-row">
        <span class="setting-label">周期性刷新事件流数据</span>
        <n-switch v-model:value="pollEnabled" size="small" @update:value="savePollConfig" />
      </div>
      <div class="setting-row">
        <span class="setting-label">刷新周期</span>
        <n-input-number
          v-model:value="pollInterval"
          size="small"
          :min="1"
          :max="60"
          :disabled="!pollEnabled"
          style="width: 110px"
          @update:value="savePollConfig"
        >
          <template #suffix>秒</template>
        </n-input-number>
      </div>
      <div class="setting-row">
        <span class="setting-value hint-text">
          仅作用于事件流界面：每个会话只保留最新 50 条事件，且只重新解析有更新的日志文件。
          历史会话不参与自动刷新，进入界面时获取列表，点开会话时加载详情。
        </span>
      </div>
    </n-card>

    <n-card title="服务器" size="small" class="settings-card">
      <div class="setting-row">
        <span class="setting-label">Web 访问地址</span>
        <a :href="serverUrl" target="_blank" rel="noopener" class="link">{{ serverUrl }}</a>
      </div>
      <div class="setting-row">
        <span class="setting-label">端口冲突处理</span>
        <span class="setting-value">启动时检测，冲突则报错</span>
      </div>
    </n-card>

    <n-card title="数据目录" size="small" class="settings-card">
      <div class="setting-row">
        <span class="setting-label">Claude 数据目录</span>
        <n-input-group style="width: 420px">
          <n-input
            v-model:value="claudeDir"
            size="small"
            placeholder="留空使用默认 ~/.claude"
            clearable
            @keyup.enter="saveClaudeDir"
          />
          <n-button size="small" :loading="claudeDirSaving" @click="saveClaudeDir">
            {{ claudeDirSaved ? "已保存" : "保存" }}
          </n-button>
        </n-input-group>
      </div>
      <div class="setting-row">
        <span class="setting-label">软件数据目录</span>
        <span class="setting-value">~/.ccbuddy/data</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">日志源目录</span>
        <span class="setting-value">{{ eventsDir || "~/.ccbuddy/events" }}</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">配置文件</span>
        <span class="setting-value">~/.ccbuddy/config.json</span>
      </div>
    </n-card>
  </div>
</template>

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
  padding: 8px 0;
  border-bottom: 1px solid var(--border);
}
.setting-row:last-child {
  border-bottom: none;
}
.setting-label {
  color: var(--text-2);
  font-size: 13px;
  flex-shrink: 0;
}
.setting-value {
  color: var(--text-1);
  font-size: 13px;
  text-align: right;
}
.hint-text {
  max-width: 460px;
  color: var(--text-3);
  font-size: 12px;
  line-height: 1.6;
}
.hint-text a,
.link {
  color: var(--accent);
  text-decoration: none;
}
</style>
