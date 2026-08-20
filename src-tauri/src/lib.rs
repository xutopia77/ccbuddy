mod event;
mod server;
mod state;

use std::path::Path;

use serde_json::{json, Value};
use tauri::Manager;

use state::SessionInfo;

/// 返回所有会话（从 `~/.claude/data/events` 日志目录解析）。
#[tauri::command]
fn get_sessions() -> Vec<SessionInfo> {
    state::load_sessions()
}

/// 返回日志源目录路径（供前端/设置页展示）。
#[tauri::command]
fn get_events_dir() -> String {
    state::events_dir().to_string_lossy().to_string()
}

/// 一键安装：把 `ccbuddy-hook` 复制到 `~/.claude/`，并在 `settings.json` 注册 hooks。
#[tauri::command]
fn install_hooks(app: tauri::AppHandle) -> Result<String, String> {
    let hook_name = if cfg!(windows) {
        "ccbuddy-hook.exe"
    } else {
        "ccbuddy-hook"
    };

    // 1. 定位 hook 源：resource_dir（release 安装后）→ 主程序同目录（开发时）
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(rd) = app.path().resource_dir() {
        candidates.push(rd.join(hook_name));
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(hook_name));
        }
    }
    let hook_src = candidates
        .into_iter()
        .find(|p| p.exists())
        .ok_or_else(|| "未找到 hook 程序 ccbuddy-hook，请重新安装或重新构建".to_string())?;

    // 2. 复制到 ~/.claude/
    let home = dirs::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
    let claude_dir = home.join(".claude");
    std::fs::create_dir_all(&claude_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let hook_dst = claude_dir.join(hook_name);
    std::fs::copy(&hook_src, &hook_dst).map_err(|e| format!("复制 hook 失败: {e}"))?;

    // 3. 写 settings.json 注册 hooks
    write_hook_settings(&claude_dir, &hook_dst)?;

    Ok(format!("已安装 hook 并注册到 {}", claude_dir.display()))
}

/// 合并写入 `~/.claude/settings.json` 的 hooks 配置，保留原有其他字段。
fn write_hook_settings(claude_dir: &Path, hook_path: &Path) -> Result<(), String> {
    let settings_path = claude_dir.join("settings.json");
    // Windows 上 hook command 由 Git Bash 执行，正斜杠路径更可靠（避免反斜杠转义问题）
    let command = hook_path.to_string_lossy().replace('\\', "/");

    // 读取现有配置；若含注释（jsonc）无法解析，则先备份原文件再覆盖
    let (mut root, backed_up) = match std::fs::read_to_string(&settings_path) {
        Ok(text) => match serde_json::from_str::<Value>(&text) {
            Ok(v) => (v, false),
            Err(_) => {
                let _ = std::fs::copy(&settings_path, claude_dir.join("settings.json.bak"));
                (Value::Object(Default::default()), true)
            }
        },
        Err(_) => (Value::Object(Default::default()), false),
    };

    if let Value::Object(map) = &mut root {
        merge_hooks(map, &command);
    }

    let text = serde_json::to_string_pretty(&root).map_err(|e| format!("序列化失败: {e}"))?;
    std::fs::write(&settings_path, text).map_err(|e| format!("写入 settings.json 失败: {e}"))?;

    if backed_up {
        return Ok(());
    }
    Ok(())
}

/// 把 ccbuddy-hook 的 hook 合并进现有 hooks 配置，不覆盖已有事件与其他 hook。
fn merge_hooks(map: &mut serde_json::Map<String, Value>, command: &str) {
    let hook_events = [
        "PreToolUse",
        "PostToolUse",
        "Notification",
        "UserPromptSubmit",
        "Stop",
        "SubagentStop",
        "SessionStart",
        "SessionEnd",
    ];
    let entry = json!({
        "matcher": "*",
        "hooks": [
            { "type": "command", "command": command }
        ]
    });

    // 获取或创建 hooks 对象（保留已有 hook 配置）
    let hooks = map
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Value::Object(hooks_map) = hooks {
        for ev in hook_events {
            let arr = hooks_map
                .entry(ev.to_string())
                .or_insert_with(|| Value::Array(Vec::new()));
            if let Value::Array(arr_vec) = arr {
                // 去重：该事件下已有指向相同 command 的 entry 时跳过
                if !arr_vec.iter().any(|e| entry_has_command(e, command)) {
                    arr_vec.push(entry.clone());
                }
            }
        }
    }
}

/// 判断一个 hook entry 是否已指向指定 command（兼容扁平与三层两种格式）。
fn entry_has_command(entry: &Value, command: &str) -> bool {
    if entry.get("command").and_then(|v| v.as_str()) == Some(command) {
        return true;
    }
    entry
        .get("hooks")
        .and_then(|v| v.as_array())
        .map(|hs| {
            hs.iter()
                .any(|h| h.get("command").and_then(|v| v.as_str()) == Some(command))
        })
        .unwrap_or(false)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![
            get_sessions,
            get_events_dir,
            install_hooks
        ])
        .setup(|_app| {
            tauri::async_runtime::spawn(async {
                server::start().await;
            });
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_hooks_preserves_existing() {
        // 已有配置：一个其他 hook（PreToolUse）+ 一个无关字段 env
        let mut map = serde_json::Map::new();
        map.insert("env".to_string(), json!({ "FOO": "bar" }));
        map.insert(
            "hooks".to_string(),
            json!({
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "other-hook" }] }
                ]
            }),
        );

        merge_hooks(&mut map, "C:/Users/x/.claude/ccbuddy-hook.exe");

        let hooks = map.get("hooks").unwrap().as_object().unwrap();
        // 原有其他 hook 保留，并追加 ccbuddy-hook
        let pre = hooks["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 2, "PreToolUse 应保留原 hook 并追加 ccbuddy-hook");
        // 无关字段保留
        assert!(map.contains_key("env"), "env 字段应保留");
        // 8 个事件全部注册
        for ev in [
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "UserPromptSubmit",
            "Stop",
            "SubagentStop",
            "SessionStart",
            "SessionEnd",
        ] {
            assert!(hooks.contains_key(ev), "缺少事件 {ev}");
        }
    }

    #[test]
    fn merge_hooks_dedupes() {
        let mut map = serde_json::Map::new();
        map.insert(
            "hooks".to_string(),
            json!({
                "PreToolUse": [
                    { "matcher": "*", "hooks": [{ "type": "command", "command": "C:/Users/x/.claude/ccbuddy-hook.exe" }] }
                ]
            }),
        );

        merge_hooks(&mut map, "C:/Users/x/.claude/ccbuddy-hook.exe");

        let pre = map["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "相同 command 不应重复添加");
    }
}
