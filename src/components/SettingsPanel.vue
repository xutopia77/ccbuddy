<script setup lang="ts">
import { installHooks as callInstallHooks } from "../api";

defineProps<{ eventsDir: string }>();

async function installHooks() {
  try {
    const msg = await callInstallHooks();
    alert(`✅ ${msg}`);
  } catch (e) {
    alert(`❌ 安装失败：${e}`);
  }
}
</script>

<template>
  <div class="settings-panel">
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
        <span class="setting-value">{{ eventsDir || '~/.ccbuddy/events' }}</span>
      </div>
    </div>
  </div>
</template>