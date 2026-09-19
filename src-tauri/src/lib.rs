//! ccbuddy 后端 crate（库）根：模块注册 + hook 内嵌/安装逻辑。

pub mod claude;
mod utils;
mod auth;
mod config;
pub mod core;
mod event;
mod events_mgr;
pub mod logger;
#[cfg(feature = "gui")]
mod notify;
mod proto;
mod server;
mod usage_mgr;
mod watch;

// 对外暴露 RPC 响应类型：`core::dispatch` 的返回值（集成测试用）。
// proto 模块本身保持私有，只把这一个类型提出来，避免扩大公开面。
pub use proto::RpcResponse;

use serde_json::{json, Value};

/// 内嵌 hook 二进制（build.py 先编 hook 再编主程序时有内容；
/// 裸 cargo build 时为 0 字节占位，运行期判空走手动放置提示）。
static EMBEDDED_HOOK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/embedded-hook.bin"));

/// 无头服务入口：启动内嵌 HTTP 服务（无桌面环境的 Linux 服务器使用）。
///
/// assets 传入编译时嵌入的前端静态资源（include_dir）。
pub fn run_server(addr: &str, assets: &'static include_dir::Dir<'static>) {
    let _ = logger::init(logger::Config::default());
    // 启动清理过期事件日志（幂等，只删 events 目录下超过保留期的 event-*.jsonl）
    events_mgr::cleanup_old_event_logs();
    // 后台预热历史会话概要缓存，避免首次打开历史视图时集中解析卡顿
    claude::prewarm_async();
    // 后台预热量用记录缓存，避免首次打开用量视图时集中解析卡顿
    usage_mgr::prewarm_async();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("创建 tokio 运行时失败");
    rt.block_on(server::start(addr, assets));
}

/// server 监听地址解析（供 ccbuddy-server bin 使用）：
/// CCBUDDY_ADDR 环境变量 > 配置文件端口（绑定地址按密码策略）。
pub fn config_server_addr() -> String {
    config::server_addr()
}

/// 生成密码的 Argon2id 哈希（`--set-password` 入口用）。
pub fn hash_password(plain: &str) -> Result<String, String> {
    config::hash_password(plain)
}

/// 写入 server 访问密码（Some = 哈希值，None = 清空）。
pub fn write_server_password(hashed: Option<&str>) -> Result<(), String> {
    config::write_server_password(hashed)
}

/// 配置文件路径的显示形式（提示信息用）。
pub fn config_path_display() -> String {
    config::config_path().to_string_lossy().to_string()
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

/// 当前平台 hook 的历史发布名：`ccbuddy-hook-<platform>-<arch>[.exe]`。
/// 现发布包内统一用裸名 `ccbuddy-hook`，此名仅作为旧版便携包的兼容候选保留。
pub fn hook_release_file_name() -> String {
    let (plat, arch) = platform_ident();
    let ext = if cfg!(windows) { ".exe" } else { "" };
    format!("ccbuddy-hook-{plat}-{arch}{ext}")
}

/// hook 的本地候选文件名：标准名 + 平台命名（便携包附带平台命名版本）。
pub fn hook_candidate_names() -> Vec<String> {
    vec![hook_file_name().to_string(), hook_release_file_name()]
}

/// 把内嵌 hook 解出到 `~/.ccbuddy/bin/`，返回解出后的路径。
fn extract_embedded_hook() -> Result<std::path::PathBuf, String> {
    let bin_dir = config::data_root().join("bin");
    std::fs::create_dir_all(&bin_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let dst = bin_dir.join(hook_file_name());
    std::fs::write(&dst, EMBEDDED_HOOK).map_err(|e| format!("解出内嵌 hook 失败: {e}"))?;

    // 非 Windows 平台需要可执行权限
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dst, std::fs::Permissions::from_mode(0o755));
    }

    log::info!("内嵌 hook 已解出: {}", dst.display());
    Ok(dst)
}

/// 安装 hook 的共享实现：本地候选 → 内嵌解出 → 复制到 Claude 目录并注册 hooks。
/// Claude 目录用户可配置（`~/.ccbuddy/config.json`），默认 `~/.claude`。
pub fn install_hooks_with(candidates: Vec<std::path::PathBuf>) -> Result<String, String> {
    let hook_name = hook_file_name();

    // 本地候选：调用方提供的路径 + ~/.ccbuddy/bin（手动放置的位置）
    let mut all_candidates = candidates;
    let manual_dir = config::data_root().join("bin");
    for name in hook_candidate_names() {
        all_candidates.push(manual_dir.join(&name));
    }

    // 候选链：
    // 1. 本地候选（resource_dir / 主程序同目录 / ~/.ccbuddy/bin，标准名或平台命名）：
    //    找第一个存在且非空的文件
    // 2. 程序内嵌 hook：解出到 ~/.ccbuddy/bin/（0 字节占位视为无内嵌，跳过）
    // 3. 都没有 → 报错，提示从发布包获取后手动放置
    let hook_src: Option<std::path::PathBuf> = all_candidates.into_iter().find(|p| {
        p.is_file() && p.metadata().map(|m| m.len() > 0).unwrap_or(false)
    });

    let hook_src = match hook_src {
        Some(p) => p,
        None => {
            if EMBEDDED_HOOK.is_empty() {
                return Err(format!(
                    "程序内未内嵌 hook 且本地未找到，请从发布包获取 {} 放入 {} 后重试",
                    hook_name,
                    config::data_root().join("bin").display(),
                ));
            }
            extract_embedded_hook()?
        }
    };

    // 复制到 Claude 目录
    let claude_dir = config::claude_dir();
    std::fs::create_dir_all(&claude_dir).map_err(|e| format!("创建目录失败: {e}"))?;
    let hook_dst = claude_dir.join(hook_name);
    std::fs::copy(&hook_src, &hook_dst).map_err(|e| format!("复制 hook 失败: {e}"))?;
    // Unix：copy 不保留可执行位，显式补上（Windows 无此概念，跳过）
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&hook_dst, std::fs::Permissions::from_mode(0o755));
    }

    // 写 settings.json 注册 hooks
    write_hook_settings(&claude_dir, &hook_dst)?;

    Ok(format!("已安装 hook 并注册到 {}", claude_dir.display()))
}

/// 卸载 hook：从 `settings.json` 各事件数组移除指向 ccbuddy-hook 的 entry
/// （保留用户其他 hooks），并删除已复制到 Claude 目录的 hook 二进制。
pub fn uninstall_hooks_with() -> Result<String, String> {
    let hook_name = hook_file_name();
    let claude_dir = config::claude_dir();
    let hook_dst = claude_dir.join(hook_name);

    // settings.json 存在才需要清理注册
    let settings_path = claude_dir.join("settings.json");
    if settings_path.is_file() {
        // 读取现有配置：文件存在但解析失败时中止（不覆盖，与安装路径同策略）
        let mut root: Value = match std::fs::read_to_string(&settings_path) {
            Ok(text) => serde_json::from_str::<Value>(&text).map_err(|e| {
                format!(
                    "settings.json 解析失败，已中止卸载（未做任何修改），请先修复后重试：{e}"
                )
            })?,
            Err(e) => return Err(format!("读取 settings.json 失败: {e}")),
        };
        let mut changed = false;
        if let Value::Object(map) = &mut root {
            let hook_path = hook_dst.to_string_lossy().replace('\\', "/");
            changed = remove_hook_entries(map, &hook_path);
        }
        // 有移除才写回（无注册时不动文件，避免无谓的时间戳变化）
        if changed {
            let text = serde_json::to_string_pretty(&root)
                .map_err(|e| format!("序列化失败: {e}"))?;
            write_settings_atomic(&settings_path, &text)?;
        }
    }

    // 删除已复制的 hook 二进制（不存在/删失败不报错，只 warn）
    if hook_dst.exists() {
        if let Err(e) = std::fs::remove_file(&hook_dst) {
            log::warn!("删除 hook 文件失败（忽略）: {}: {e}", hook_dst.display());
        }
    }

    Ok(format!("已卸载 hook（若此前已安装于 {}）", claude_dir.display()))
}

/// 读取并解析 `settings.json`：文件不存在 → 空对象（首次安装路径）；
/// 存在但解析失败（jsonc 注释/损坏/截断）→ **报错中止**，绝不从空对象重建覆盖
/// （否则 permissions/env/statusLine/其他 hooks 全部丢失）。
fn read_settings_or_err(settings_path: &std::path::Path) -> Result<Value, String> {
    match std::fs::read_to_string(settings_path) {
        Ok(text) => serde_json::from_str::<Value>(&text).map_err(|e| {
            format!(
                "settings.json 解析失败，已中止安装（未修改任何文件），请先修复后重试：{e}"
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // 文件不存在：从空对象新建（正常首次安装）
            Ok(Value::Object(Default::default()))
        }
        Err(e) => Err(format!("读取 settings.json 失败: {e}")),
    }
}

/// 原子写 `settings.json`：先写同目录临时文件，再 rename 替换。
/// 避免中途崩溃留下截断文件导致 Claude Code 报 Settings Error 且 hooks 全失效。
fn write_settings_atomic(settings_path: &std::path::Path, text: &str) -> Result<(), String> {
    let tmp_path = settings_path.with_extension("json.tmp");
    std::fs::write(&tmp_path, text).map_err(|e| format!("写入临时文件失败: {e}"))?;
    // Windows 上 std::fs::rename 覆盖已存在目标（内部 MoveFileEx + REPLACE_EXISTING）；
    // 若目标被占用导致 rename 失败，退化为 remove 后 rename（窗口极小，可接受）
    if let Err(e) = std::fs::rename(&tmp_path, settings_path) {
        let _ = std::fs::remove_file(settings_path);
        std::fs::rename(&tmp_path, settings_path).map_err(|e2| {
            let _ = std::fs::remove_file(&tmp_path); // 清理残留临时文件
            format!("替换 settings.json 失败: {e} / {e2}")
        })?;
    }
    Ok(())
}

/// 读取并解析 `settings.json`，合并 ccbuddy hooks 后原子写回（保留原有其他字段）。
fn write_hook_settings(claude_dir: &std::path::Path, hook_path: &std::path::Path) -> Result<(), String> {
    let settings_path = claude_dir.join("settings.json");
    // Windows 上 hook command 由 Git Bash 执行，正斜杠路径更可靠（避免反斜杠转义问题）；
    // 路径加双引号，避免用户名等含空格时 shell 无法执行
    let command = format!("\"{}\"", hook_path.to_string_lossy().replace('\\', "/"));

    let mut root: Value = read_settings_or_err(&settings_path)?;

    if let Value::Object(map) = &mut root {
        merge_hooks(map, &command);
    }

    let text = serde_json::to_string_pretty(&root).map_err(|e| format!("序列化失败: {e}"))?;
    write_settings_atomic(&settings_path, &text)?;

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
    use crate::notify;
    use crate::proto::{RpcRequest, RpcResponse};

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
            .setup(|app| {
                // 任务栏通知状态（已读 / 已闪烁集合）
                app.manage(notify::NotifyState::default());

                // 启动清理过期事件日志（幂等，L-B15）
                crate::events_mgr::cleanup_old_event_logs();

                // 后台预热历史会话概要缓存，避免首次打开历史视图时集中解析卡顿
                crate::claude::prewarm_async();
                // 后台预热量用记录缓存，避免首次打开用量视图时集中解析卡顿
                crate::usage_mgr::prewarm_async();

                // 事件流变更推送：watcher 检测到新事件时 emit 给前端
                // （信封 = RpcResponse，与 ccbuddy-server 的 SSE 推送格式一致）
                use tauri::Emitter;
                if let Some(window) = app.get_webview_window("main") {
                    let win = window.clone();
                    let guard = crate::events_mgr::subscribe(Box::new(move |sessions| {
                        let resp = crate::proto::ok("events_changed", serde_json::json!(sessions));
                        let _ = win.emit("events_changed", resp);
                    }));
                    // guard 故意 forget：与 AppHandle 同生命周期（应用常驻，监听不注销）
                    std::mem::forget(guard);

                    // 用量变更推送：同一信封协议，cmd = usage_changed
                    let win2 = window.clone();
                    let guard2 = crate::usage_mgr::subscribe_usage(Box::new(move |records| {
                        let resp = crate::proto::ok("usage_changed", serde_json::json!(records));
                        let _ = win2.emit("usage_changed", resp);
                    }));
                    std::mem::forget(guard2);
                }

                // 后台每 2s 轮询事件流目录（不依赖前端页面），驱动任务栏角标与闪烁
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    let mut tick = tokio::time::interval(notify::POLL_INTERVAL);
                    loop {
                        tick.tick().await;
                        // 事件解析是阻塞 IO，放到阻塞线程池执行
                        let urgent = tauri::async_runtime::spawn_blocking(
                            crate::events_mgr::urgent_session_ids,
                        )
                        .await
                        .unwrap_or_default();
                        notify::poll(&handle, urgent);
                    }
                });
                Ok(())
            })
            .on_window_event(|window, event| {
                // 打开软件恢复常态：聚焦时清空角标 + 当前紧急会话记入已读 + 清空已闪烁集合
                if let tauri::WindowEvent::Focused(focused) = event {
                    let app = window.app_handle();
                    let state = app.state::<notify::NotifyState>();
                    state.set_focused(*focused);
                    if *focused {
                        notify::on_focus(app);
                    }
                }
            })
            .invoke_handler(tauri::generate_handler![rpc])
            .run(tauri::generate_context!())
            .expect("error while running tauri application");
    }
}

#[cfg(feature = "gui")]
pub use gui::run;

/// ccbuddy-hook 注册的 Claude Code Hook 事件列表（共 33 个：工具生命周期 / 权限 /
/// 会话 / 通知 / 文件与工作区 / 子代理与任务 / 压缩 / MCP 交互 / 初始化 / 模型切换）。
///
/// 已知取舍：每个事件发生时 Claude Code 都会 spawn 一个 hook 进程（毫秒级完成），
/// 高频事件（PreToolUse/PostToolUse/MessageDisplay）会为每次工具调用增加约
/// 5-20ms 开销。注册全集是为了状态机扩展时无需重装 hook。
pub const HOOK_EVENTS: [&str; 33] = [
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "PostToolBatch",
    "PermissionRequest",
    "PermissionDenied",
    "Notification",
    "MessageDisplay",
    "UserPromptSubmit",
    "UserPromptExpansion",
    "Stop",
    "StopFailure",
    "SubagentStart",
    "SubagentStop",
    "TaskCreated",
    "TaskCompleted",
    "TeammateIdle",
    "PreModelSwitch",
    "PostModelSwitch",
    "PreCompact",
    "PostCompact",
    "Elicitation",
    "ElicitationResult",
    "ConfigChange",
    "CwdChanged",
    "DirectoryAdded",
    "FileChanged",
    "InstructionsLoaded",
    "WorktreeCreate",
    "WorktreeRemove",
    "Setup",
    "SessionStart",
    "SessionEnd",
];

/// 把 ccbuddy-hook 的 hook 合并进现有 hooks 配置，不覆盖已有事件与其他 hook。
fn merge_hooks(map: &mut serde_json::Map<String, Value>, command: &str) {
    // 工具类事件的 matcher 支持通配 "*"；SessionStart/SessionEnd 的 matcher
    // 取值是枚举（startup|resume|clear|compact|fork / clear|resume|logout|…），
    // "*" 无全匹配保证——官方允许省略 matcher（=全匹配），这两个事件省略之。
    let entry_with_matcher = json!({
        "matcher": "*",
        "hooks": [
            { "type": "command", "command": command }
        ]
    });
    let entry_without_matcher = json!({
        "hooks": [
            { "type": "command", "command": command }
        ]
    });

    // 获取或创建 hooks 对象（保留已有 hook 配置）
    let hooks = map
        .entry("hooks".to_string())
        .or_insert_with(|| Value::Object(serde_json::Map::new()));

    if let Value::Object(hooks_map) = hooks {
        for ev in HOOK_EVENTS {
            let entry = if matches!(ev, "SessionStart" | "SessionEnd") {
                &entry_without_matcher
            } else {
                &entry_with_matcher
            };
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

/// 卸载用：从 hooks 配置中移除所有 command 指向 `hook_path`（去引号、正斜杠形态）
/// 的 entry，保留用户其他 hooks；返回是否有任何移除（决定是否写回文件）。
/// 判定复用 [`entry_has_command`]（兼容扁平/三层、带引号/裸路径）。
fn remove_hook_entries(map: &mut serde_json::Map<String, Value>, hook_path: &str) -> bool {
    let hooks = match map.get_mut("hooks") {
        Some(Value::Object(hooks_map)) => hooks_map,
        _ => return false,
    };

    let mut changed = false;
    for arr in hooks.values_mut() {
        let Value::Array(arr_vec) = arr else { continue };
        let before = arr_vec.len();
        // 逐条判定是否为 ccbuddy 注册
        arr_vec.retain(|e| !entry_has_command(e, hook_path));
        if arr_vec.len() != before {
            changed = true;
        }
    }
    // 清掉移除后变空的数组，避免留下 "Event": [] 空壳
    if changed {
        hooks.retain(|_, v| !matches!(v, Value::Array(a) if a.is_empty()));
    }
    changed
}

/// 去掉 command 首尾引号（新写入带引号，旧注册可能不带，比较时统一归一）。
fn normalized_command(s: &str) -> &str {
    s.trim().trim_matches('"')
}

/// 判断一个 hook entry 是否已指向指定 command（兼容扁平与三层两种格式，
/// 及带引号/不带引号两种写法）。
pub fn entry_has_command(entry: &Value, command: &str) -> bool {
    let want = normalized_command(command);
    // 命中判定：去引号归一后字符串相等
    let hit = |c: Option<&str>| c.map(normalized_command) == Some(want);

    // 扁平格式：{ "command": "..." }
    if hit(entry.get("command").and_then(|v| v.as_str())) {
        return true;
    }
    // 三层格式：{ "hooks": [ { "command": "..." }, ... ] }
    let nested: Option<bool> = entry
        .get("hooks")
        .and_then(|v| v.as_array())
        .map(|hs| {
            for h in hs {
                if hit(h.get("command").and_then(|v| v.as_str())) {
                    return true;
                }
            }
            false
        });
    nested.unwrap_or(false)
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
        // 全部 hook 事件均已注册
        for ev in HOOK_EVENTS {
            assert!(hooks.contains_key(ev), "缺少事件 {ev}");
        }
        // 新追加的 entry：工具类事件带 matcher:"*"，SessionStart/SessionEnd 省略 matcher
        let ours = pre.iter().find(|e| entry_has_command(e, "C:/Users/x/.claude/ccbuddy-hook.exe")).unwrap();
        assert_eq!(ours.get("matcher").and_then(|v| v.as_str()), Some("*"), "工具类事件 entry 应带 matcher:\"*\"");
        let session_start = hooks["SessionStart"].as_array().unwrap();
        let ours_ss = session_start.iter().find(|e| entry_has_command(e, "C:/Users/x/.claude/ccbuddy-hook.exe")).unwrap();
        assert!(ours_ss.get("matcher").is_none(), "SessionStart entry 应省略 matcher（枚举值事件，省略=全匹配）");
        let session_end = hooks["SessionEnd"].as_array().unwrap();
        let ours_se = session_end.iter().find(|e| entry_has_command(e, "C:/Users/x/.claude/ccbuddy-hook.exe")).unwrap();
        assert!(ours_se.get("matcher").is_none(), "SessionEnd entry 应省略 matcher（枚举值事件，省略=全匹配）");
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

    #[test]
    fn merge_hooks_dedupes_quoted_vs_bare() {
        // 旧版注册为裸路径，新版写入带引号：视为同一条，不重复添加
        let mut map = serde_json::Map::new();
        map.insert(
            "hooks".to_string(),
            json!({
                "PreToolUse": [
                    { "matcher": "*", "hooks": [{ "type": "command", "command": "C:/Users/John Doe/.claude/ccbuddy-hook.exe" }] }
                ]
            }),
        );

        merge_hooks(&mut map, "\"C:/Users/John Doe/.claude/ccbuddy-hook.exe\"");

        let pre = map["hooks"]["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "带引号与裸路径应视为同一 command");
    }

    #[test]
    fn uninstall_removes_only_ours() {
        // 混合用户 hooks + ccbuddy hooks 的 settings → 卸载后仅 ccbuddy entry 被移除
        let ccbuddy = "C:/Users/x/.claude/ccbuddy-hook.exe";
        let mut map = serde_json::Map::new();
        map.insert("env".to_string(), json!({ "FOO": "bar" }));
        map.insert(
            "hooks".to_string(),
            json!({
                // 混合：ccbuddy + 用户 hook（应只剩用户 hook）
                "PreToolUse": [
                    { "matcher": "Bash", "hooks": [{ "type": "command", "command": "other-hook" }] },
                    { "matcher": "*", "hooks": [{ "type": "command", "command": ccbuddy }] }
                ],
                // 仅 ccbuddy（应整组移除，不留空数组）
                "SessionEnd": [
                    { "hooks": [{ "type": "command", "command": ccbuddy }] }
                ]
            }),
        );

        assert!(remove_hook_entries(&mut map, ccbuddy), "应发生移除");

        let hooks = map["hooks"].as_object().unwrap();
        let pre = hooks["PreToolUse"].as_array().unwrap();
        assert_eq!(pre.len(), 1, "ccbuddy entry 被移除，用户 hook 保留");
        assert_eq!(
            pre[0]["hooks"][0]["command"].as_str(),
            Some("other-hook"),
            "保留的应是用户 hook"
        );
        assert!(!hooks.contains_key("SessionEnd"), "仅含 ccbuddy 的事件组应整体移除");
        assert!(map.contains_key("env"), "无关字段应保留");

        // 再跑一遍：已无 ccbuddy entry，返回 false（无变化不写回）
        assert!(!remove_hook_entries(&mut map, ccbuddy));

        // 带引号写法（install 实际写入形态）同样命中
        let mut map2 = serde_json::Map::new();
        map2.insert(
            "hooks".to_string(),
            json!({ "Stop": [ { "hooks": [{ "type": "command", "command": ccbuddy }] } ] }),
        );
        assert!(remove_hook_entries(&mut map2, &format!("\"{ccbuddy}\"")));
        assert!(map2["hooks"].as_object().unwrap().is_empty(), "带引号路径也应命中移除");
    }
}
