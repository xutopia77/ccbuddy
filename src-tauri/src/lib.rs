mod config;
mod core;
mod event;
mod logger;
mod rpc;
mod server;
mod state;

use serde_json::{json, Value};

/// 无头服务入口：启动内嵌 HTTP 服务（无桌面环境的 Linux 服务器使用）。
///
/// assets 传入编译时嵌入的前端静态资源（include_dir）。
pub fn run_server(addr: &str, assets: &'static include_dir::Dir<'static>) {
    let _ = logger::init(logger::Config::default());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建 tokio 运行时失败");
    rt.block_on(server::start(addr, assets));
}

/// ccbuddy-hook 可执行文件名（按平台区分）。
pub fn hook_file_name() -> &'static str {
    if cfg!(windows) {
        "ccbuddy-hook.exe"
    } else {
        "ccbuddy-hook"
    }
}

/// 当前平台标识（与打包脚本命名约定一致）：(platform, arch)。
pub fn platform_ident() -> (&'static str, &'static str) {
    let plat = if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "linux") {
        "linux"
    } else if cfg!(target_os = "macos") {
        "darwin"
    } else {
        "unknown"
    };
    let arch = std::env::consts::ARCH; // x86_64 / aarch64
    (plat, arch)
}

/// 当前平台 hook 的发布文件名：`ccbuddy-hook-<platform>-<arch>[.exe]`。
/// 与打包脚本输出（GitHub Release 附件）命名一致，可用于自动下载。
pub fn hook_release_file_name() -> String {
    let (plat, arch) = platform_ident();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    format!("ccbuddy-hook-{plat}-{arch}{ext}")
}

/// hook 的本地候选文件名：标准名 + 平台命名（便携包附带平台命名版本）。
pub fn hook_candidate_names() -> Vec<String> {
    vec![hook_file_name().to_string(), hook_release_file_name()]
}

/// hook 的 GitHub latest release 下载地址（产物名不带版本号，latest 链接固定）。
pub fn hook_download_url() -> String {
    let repo = config::load().github_repo;
    let repo = if repo.is_empty() {
        default_repo()
    } else {
        repo
    };
    format!("https://github.com/{repo}/releases/latest/download/{}", hook_release_file_name())
}

fn default_repo() -> String {
    "xutopia77/ccbuddy".to_string()
}

/// 自动下载 hook 到临时文件，返回下载后的路径。
fn download_hook() -> Result<std::path::PathBuf, String> {
    use std::io::Read;

    let url = hook_download_url();
    log::info!("本地未找到 hook，尝试自动下载: {url}");

    let resp = ureq::get(&url)
        .timeout(std::time::Duration::from_secs(60))
        .call()
        .map_err(|e| format!("下载失败（离线或网络受限）: {e}\n下载地址: {url}"))?;

    let mut reader = resp
        .into_reader()
        .take(64 * 1024 * 1024); // 上限 64MB 防异常响应

    let dst = std::env::temp_dir().join(hook_release_file_name());
    let mut file = std::fs::File::create(&dst).map_err(|e| format!("创建临时文件失败: {e}"))?;
    std::io::copy(&mut reader, &mut file).map_err(|e| format!("写入文件失败: {e}"))?;

    // 非Windows 平台需要可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755));
    }

    log::info!("hook 下载完成: {}", dst.display());
    Ok(dst)
}

/// 安装 hook 的共享实现：本地候选 → 自动下载 → 复制到 Claude 目录并注册 hooks。
/// Claude 目录用户可配置（`~/.ccbuddy/config.json`），默认 `~/.claude`。
pub fn install_hooks_with(candidates: Vec<std::path::PathBuf>) -> Result<String, String> {
    let hook_name = hook_file_name();

    // 本地候选：调用方提供的路径 + ~/.ccbuddy/bin（手动下载的放置位置）
    let mut all_candidates = candidates;
    let manual_dir = config::data_root().join("bin");
    for name in hook_candidate_names() {
        all_candidates.push(manual_dir.join(&name));
    }

    // 1. 本地候选（resource_dir / 主程序同目录，标准名或平台命名）
    let hook_src = all_candidates.into_iter().find(|p| {
        p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
    });

    // 2. 本地没有 → 从 GitHub latest release 自动下载
    let (hook_src, downloaded) = match hook_src {
        Some(p) => (p, false),
        None => match download_hook() {
            Ok(p) => (p, true),
            Err(e) => {
                let claude = config::claude_dir();
                return Err(format!(
                    "{e}\n\n离线环境请手动处理：\n\
                     1. 浏览器打开上面的下载地址下载 {}\n\
                     2. 将文件放到 {} 目录下（或主程序同目录），重新点击安装\n\
                     3. 或将文件重命名为 {hook_name} 直接放入 {}",
                    hook_release_file_name(),
                    config::data_root().join("bin").display(),
                    claude.display(),
                ));
            }
        },
    };

    // 复制到 Claude 目录
    let claude_dir = config::claude_dir();
    std::fs::create_dir_all(&claude_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let hook_dst = claude_dir.join(hook_name);
    std::fs::copy(&hook_src, &hook_dst).map_err(|e| format!("复制 hook 失败: {e}"))?;

    // 写 settings.json 注册 hooks
    write_hook_settings(&claude_dir, &hook_dst)?;

    if downloaded {
        // 下载的临时文件用完清理
        let _ = std::fs::remove_file(&hook_src);
    }

    Ok(format!(
        "已安装 hook 并注册到 {}{}",
        claude_dir.display(),
        if downloaded { "（hook 自动下载完成）" } else { "" }
    ))
}

/// 合并写入 `~/.claude/settings.json` 的 hooks 配置，保留原有其他字段。
fn write_hook_settings(claude_dir: &std::path::Path, hook_path: &std::path::Path) -> Result<(), String> {
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

/// 桌面 GUI（Tauri）相关代码，仅启用 `gui` feature 时编译。
///
/// 这是唯一的 Tauri 适配层：把 Tauri 的 `invoke` 调用桥接到 [`crate::core::dispatch`]。
/// 业务逻辑全部在 `core` / `state` / `event` 等纯 Rust 模块中，不接触 Tauri。
#[cfg(feature = "gui")]
mod gui {
    use tauri::Manager;

    use crate::core::{self, RpcContext};
    use crate::rpc::{RpcRequest, RpcResponse};

    /// 统一 RPC 入口：前端 `invoke("rpc", { payload })` 的唯一落点。
    #[tauri::command]
    fn rpc(app: tauri::AppHandle, payload: RpcRequest) -> RpcResponse {
        let ctx = build_context(&app);
        core::dispatch(&ctx, &payload.cmd, payload.data)
    }

    /// 构造 GUI 环境上下文：hook 候选源 = resource_dir + 主程序同目录。
    fn build_context(app: &tauri::AppHandle) -> RpcContext {
        let mut candidates: Vec<std::path::PathBuf> = Vec::new();
        let names = crate::hook_candidate_names();
        if let Ok(rd) = app.path().resource_dir() {
            for name in &names {
                candidates.push(rd.join(name));
            }
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                for name in &names {
                    candidates.push(dir.join(name));
                }
            }
        }
        RpcContext {
            hook_candidates: candidates,
        }
    }

    #[cfg_attr(mobile, tauri::mobile_entry_point)]
    pub fn run() {
        let mut logdefault = crate::logger::Config::default();
        logdefault.level = crate::logger::Level::Debug;
        let _ = crate::logger::init(logdefault);
        tauri::Builder::default()
            .plugin(tauri_plugin_opener::init())
            .invoke_handler(tauri::generate_handler![rpc])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}

#[cfg(feature = "gui")]
pub use gui::run;

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
pub fn entry_has_command(entry: &Value, command: &str) -> bool {
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
