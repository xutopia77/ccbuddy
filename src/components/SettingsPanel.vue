<script setup lang="ts">
import { onMounted, ref, computed } from "vue";
import { installHooks as callInstallHooks, getHookStatus, type HookStatus } from "../api";
import type { PollConfig } from "../types";

defineProps<{ eventsDir: string }>();

// ---- hook 状态 ----
const hookStatus = ref<HookStatus | null>(null);

async function refreshHookStatus() {
  try {
    hookStatus.value = await getHookStatus();
  } catch (e) {
    console.error("获取 hook 状态失败", e);
  }
}

async function installHooks() {
  try {
    const msg = await callInstallHooks();
    alert(`✅ ${msg}`);
    await refreshHookStatus();
  } catch (e) {
    alert(`❌ 安装失败：${e}`);
  }
}

// 已注册的事件数 / 总事件数
const registeredCount = computed(() => {
  if (!hookStatus.value) return "";
  const vals = Object.values(hookStatus.value.registered);
  const ok = vals.filter(Boolean).length;
  return `${ok}/${vals.length}`;
});

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
        <button class="btn btn-primary" @click="installHooks">一键安装/更新 Hooks</button>
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
        <span class="setting-label">软件数据目录</span>
        <span class="setting-value">~/.ccbuddy/data</span>
      </div>
      <div class="setting-row">
        <span class="setting-label">日志源目录</span>
        <span class="setting-value">{{ eventsDir || '~/.ccbuddy/events' }}</span>
      </div>
    </div>
  </div>
</template>
