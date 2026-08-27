// 后端 API 适配层：桌面端（Tauri）走 invoke，浏览器（ccbuddy-server）走 HTTP。
// 两个环境调用同一组函数，界面代码无感知。

import { invoke } from "@tauri-apps/api/core";
import type { Session } from "./types";

const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function getSessions(): Promise<Session[]> {
  if (isTauri) return invoke<Session[]>("get_sessions");
  const r = await fetch("/api/sessions");
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.json();
}

export async function getEventsDir(): Promise<string> {
  if (isTauri) return invoke<string>("get_events_dir");
  const r = await fetch("/api/events_dir");
  if (!r.ok) throw new Error(`HTTP ${r.status}`);
  return r.text();
}

export async function installHooks(): Promise<string> {
  if (isTauri) return invoke<string>("install_hooks");
  const r = await fetch("/api/install_hooks", { method: "POST" });
  if (!r.ok) throw new Error(await r.text());
  return r.text();
}
