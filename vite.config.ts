import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import Components from "unplugin-vue-components/vite";
import { NaiveUiResolver } from "unplugin-vue-components/resolvers";
import pkg from "./package.json";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async () => ({
  // 软件版本：以 package.json 为唯一来源，构建期注入为全局常量 __APP_VERSION__
  // （设置页展示用；桌面版/Web 版通用，不依赖 Tauri 运行时 API）
  define: {
    __APP_VERSION__: JSON.stringify(pkg.version),
  },

  plugins: [
    vue(),
    // naive-ui 组件按需自动导入（n-button/n-card 等直接用于模板，无需手动 import）
    Components({ resolvers: [NaiveUiResolver()] }),
  ],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
