<template>
  <div class="stage">
    <!-- 左：品牌区（窄屏隐藏） -->
    <section class="hero">
      <div class="logo">
        <svg class="ic logo-ic" viewBox="0 0 24 24">
          <polygon points="13 2 3 14 12 14 11 22 21 10 12 10 13 2" />
        </svg>
      </div>
      <h1>CC Buddy</h1>
      <p class="sub">this is <em>all</em> you need</p>
    </section>

    <!-- 右：登录表单 -->
    <section class="login-wrap">
      <form class="card" @submit.prevent="submit">
        <h2>{{ t("loginTitle") }}</h2>

        <div class="field">
          <label for="login-user">{{ t("loginUsername") }}</label>
          <div class="input disabled">
            <svg class="ic" viewBox="0 0 24 24">
              <path d="M20 21v-2a4 4 0 0 0-4-4H8a4 4 0 0 0-4 4v2" />
              <circle cx="12" cy="7" r="4" />
            </svg>
            <!-- 单用户：用户名固定，不做多用户管理，故禁用 -->
            <input id="login-user" :value="USERNAME" disabled autocomplete="username" />
          </div>
        </div>

        <div class="field">
          <label for="login-pass">{{ t("loginPassword") }}</label>
          <div class="input">
            <svg class="ic" viewBox="0 0 24 24">
              <rect x="4" y="11" width="16" height="10" rx="2" />
              <path d="M8 11V7a4 4 0 0 1 8 0v4" />
            </svg>
            <input
              id="login-pass"
              v-model="password"
              type="password"
              :placeholder="t('loginEmpty')"
              autocomplete="current-password"
              autofocus
            />
          </div>
        </div>

        <p v-if="error" class="err">
          <svg class="ic err-ic" viewBox="0 0 24 24">
            <circle cx="12" cy="12" r="9" />
            <line x1="9" y1="9" x2="15" y2="15" />
            <line x1="15" y1="9" x2="9" y2="15" />
          </svg>
          {{ error }}
        </p>

        <button class="btn-login" type="submit" :disabled="busy">
          {{ busy ? t("loadingText") : t("loginBtn") }}
        </button>

        <p class="foot">CCBuddy · Event Stream Monitor</p>
      </form>
    </section>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import { login } from "../auth";
import { t } from "../i18n";

/** 固定用户名（单用户，服务端实际只校验密码）。 */
const USERNAME = "admin";

const password = ref("");
const error = ref("");
const busy = ref(false);

async function submit() {
  if (busy.value) return;
  if (!password.value) {
    error.value = t("loginEmpty");
    return;
  }
  busy.value = true;
  error.value = "";
  try {
    await login(password.value);
    // 成功后 authed 翻真，App.vue 换掉整棵子树，本组件随之卸载
  } catch (e) {
    error.value = e instanceof Error ? e.message : String(e);
    password.value = "";
  } finally {
    busy.value = false;
  }
}
</script>

<style scoped>
/* design token 见 src/styles/tokens.css（唯一样式源） */
.stage {
  flex: 1;
  min-height: 0;
  display: flex;
}

/* ===== 左：品牌区 ===== */
.hero {
  flex: 1.2;
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 0 8%;
  min-width: 0;
}
.logo {
  width: 64px;
  height: 64px;
  border-radius: 18px;
  background: var(--accent);
  display: flex;
  align-items: center;
  justify-content: center;
}
.logo-ic {
  width: 30px;
  height: 30px;
  stroke: var(--on-accent);
  fill: none;
}
.hero h1 {
  margin-top: 28px;
  font-size: 44px;
  font-weight: 700;
  letter-spacing: 0.01em;
  color: var(--text-1);
}
.hero .sub {
  margin-top: 10px;
  font-size: 16px;
  color: var(--text-3);
  font-family: var(--font-mono);
  letter-spacing: 0.02em;
}
.hero .sub em {
  font-style: normal;
  color: var(--accent);
}

/* ===== 右：登录卡片 ===== */
.login-wrap {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  border-left: 1px solid var(--border);
  background: var(--bg-surface);
}
.card {
  width: 340px;
}
.card h2 {
  font-size: 18px;
  font-weight: 650;
  color: var(--text-1);
}
.field {
  margin-top: 18px;
}
.field label {
  display: block;
  font-size: 12px;
  color: var(--text-2);
  margin-bottom: 6px;
}
.input {
  display: flex;
  align-items: center;
  gap: 9px;
  background: var(--bg-base);
  border: 1px solid var(--border);
  border-radius: var(--radius-inset);
  padding: 0 12px;
  transition: border-color 0.15s;
}
.input:focus-within {
  border-color: var(--accent);
}
.input.disabled {
  opacity: 0.65;
}
/* 通用图标：描边跟随文字色，尺寸由具体位置覆盖 */
.ic {
  width: 15px;
  height: 15px;
  flex: none;
  stroke: currentColor;
  fill: none;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.input .ic {
  color: var(--text-3);
}
.input input {
  flex: 1;
  min-width: 0;
  background: none;
  border: none;
  outline: none;
  color: var(--text-1);
  font: inherit;
  font-size: 13.5px;
  height: 40px;
  font-family: var(--font-mono);
}
.input input::placeholder {
  color: var(--text-3);
}
.input input:disabled {
  cursor: not-allowed;
}
.btn-login {
  margin-top: 24px;
  width: 100%;
  height: 42px;
  border: none;
  border-radius: var(--radius-inset);
  cursor: pointer;
  background: var(--accent);
  color: var(--on-accent);
  font: inherit;
  font-size: 14px;
  font-weight: 650;
  transition: background 0.15s;
}
.btn-login:hover:not(:disabled) {
  background: var(--accent-hover);
}
.btn-login:disabled {
  cursor: default;
  opacity: 0.7;
}
.err {
  margin-top: 12px;
  font-size: 12px;
  color: var(--danger);
  display: flex;
  align-items: center;
  gap: 6px;
}
.err-ic {
  width: 13px;
  height: 13px;
}
.foot {
  margin-top: 28px;
  font-size: 11.5px;
  color: var(--text-3);
  text-align: center;
  font-family: var(--font-mono);
}

/* 窄屏（<860px）：只留表单 */
@media (max-width: 860px) {
  .hero {
    display: none;
  }
  .login-wrap {
    border-left: none;
  }
}
</style>
