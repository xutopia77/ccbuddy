<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { installHooks as callInstallHooks, getHookStatus, getConfig, setConfig, type HookStatus } from "../api";
import type { PollConfig } from "../types";

defineProps<{ eventsDir: string }>();

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
    alert(`✅ ${msg}`);
    await refreshHookStatus();
  } catch (e) {
    // 后端错误信息含下载地址与离线手动处理步骤
    const detail = e instanceof Error ? e.message : String(e);
    alert(`❌ 安装失败：${detail}`);
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
    alert(`❌ 保存失败：${e}`);
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
    <div class="settings-section">
      <h2>Hook 配置</h2>
      <div class="setting-row">
        <span class="setting-label">Hook Logger 状态</span>
        <span>
          <span :class="hookStatus?.installed ? 'status-indicator status-ok' : 'status-indicator status-error'"></span>
          {{ hookStatus?.installed ? "已安装" : "未安装" }}
        </span>
      </div>
      <div class="setting-row">
        <span class="setting-label">可执行文件路径</span>
        <span class="setting-value">~/.claude/ccbuddy-hook</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">Claude settings.json 注册</span>
        <span>
          <span :class="registeredCount === '8/8' ? 'status-indicator status-ok' : 'status-indicator status-warn'"></span>
          {{ registeredCount || "-" }} 个事件已注册
        </span>
      </div>
      <div class="setting-row">
        <span class="setting-label">hook 安装来源</span>
        <span class="setting-value" style="max-width:420px; text-align:right;">
          安装时优先使用本地 hook（安装包内置 / 便携包同目录 / ~/.ccbuddy/bin），
          本地没有则自动从 GitHub Release 下载；离线环境可
          <a :href="hookStatus?.downloadUrl" target="_blank" rel="noopener" style="color: var(--accent);">手动下载</a>
          后放入 ~/.ccbuddy/bin/
        </span>
      </div>
      <div class="setting-row">
        <button class="btn btn-primary" :disabled="installing" @click="installHooks">
          {{ installing ? "安装中（可能正在下载 hook）..." : "一键安装/更新 Hooks" }}
        </button>
      </div>
    </div>
    <div class="settings-section">
      <h2>数据刷新</h2>
      <div class="setting-row">
        <span class="setting-label">周期性获取会话列表</span>
        <label><input type="checkbox" v-model="pollEnabled" @change="savePollConfig" /> 启用</label>
      </div>
      <div class="setting-row">
        <span class="setting-label">获取周期</span>
        <span>
          <input
            type="number"
            min="1"
            max="60"
            v-model.number="pollInterval"
            @change="savePollConfig"
            style="width:80px; background:var(--bg-tertiary); border:1px solid var(--border); color:var(--text-primary); padding:4px; border-radius:4px;"
          /> 秒
        </span>
      </div>
    </div>
    <div class="settings-section">
      <h2>服务器</h2>
      <div class="setting-row">
        <span class="setting-label">Web 访问地址</span>
        <a :href="serverUrl" target="_blank" rel="noopener" style="color: var(--accent); text-decoration: none;">{{ serverUrl }}</a>
      </div>
      <div class="setting-row">
        <span class="setting-label">端口冲突处理</span>
        <span class="setting-value">启动时检测，冲突则报错</span>
      </div>
    </div>
    <div class="settings-section">
      <h2>数据目录</h2>
      <div class="setting-row">
        <span class="setting-label">Claude 数据目录</span>
        <span class="setting-value">
          <input
            v-model="claudeDir"
            placeholder="留空使用默认 ~/.claude"
            style="width:300px; background:var(--bg-tertiary); border:1px solid var(--border); color:var(--text-primary); padding:4px 8px; border-radius:4px;"
          />
          <button class="btn btn-primary" style="margin-left:8px;" :disabled="claudeDirSaving" @click="saveClaudeDir">
            {{ claudeDirSaving ? "保存中..." : claudeDirSaved ? "已保存" : "保存" }}
          </button>
        </span>
      </div>
      <div class="setting-row">
        <span class="setting-label">软件数据目录</span>
        <span class="setting-value">~/.ccbuddy/data</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">日志源目录</span>
        <span class="setting-value">{{ eventsDir || '~/.ccbuddy/events' }}</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">配置文件</span>
        <span class="setting-value">~/.ccbuddy/config.json</span>
      </div>
    </div>
  </div>
</template>
