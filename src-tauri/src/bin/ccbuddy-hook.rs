//! `ccbuddy-hook` — Claude Code Hook 日志记录器。
//!
//! 作为 Claude Code Hooks 的调用目标：
//! - 事件类型由环境变量 `CLAUDE_HOOK_EVENT` 传入；
//! - 事件内容从 stdin 读入（JSON，原样作为 payload）；
//! - 包装 `{ received_at, hook_event, payload }` 后追加写入按会话分文件的 JSONL；
//! - 按小时轮转文件名，并在文件总数 > 50 时清理至最新 20 个。
//!
//! 设计约束：极简、只做写入、任何错误都不影响 Claude Code 主流程（退出码 0）。

use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde_json::{json, Value};

/// 日志文件数超过该值时触发清理。
const MAX_FILES: usize = 50;
/// 清理后保留的最新文件数。
const KEEP_FILES: usize = 20;

fn main() {
    // 1. 读取 stdin 全部内容作为 payload
    let mut input = String::new();
    if io::stdin().read_to_string(&mut input).is_err() {
        eprintln!("ccbuddy-hook: failed to read stdin");
        std::process::exit(0);
    }
    let payload: Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => {
            // 非 JSON 输入也原样保留为字符串，避免丢失信息
            Value::String(input)
        }
    };

    // 2. 提取事件类型：Claude Code 实际通过 stdin 的 hook_event_name 字段传入，
    //    回退到环境变量 CLAUDE_HOOK_EVENT（兼容旧版本或其他来源）。
    let hook_event = payload
        .get("hook_event_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| env::var("CLAUDE_HOOK_EVENT").unwrap_or_default());

    // 3. 提取 session_id（缺失则用 "unknown"）
    let session_id = payload
        .get("session_id")
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string();

    // 4. 构造包装对象
    let now = Utc::now();
    let entry = json!({
        "received_at": now.format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string(),
        "hook_event": hook_event,
        "payload": payload,
    });

    // 5. 追加写入按会话 + 小时命名的日志文件
    let dir = events_dir();
    if fs::create_dir_all(&dir).is_err() {
        eprintln!("ccbuddy-hook: failed to create dir {}", dir.display());
        std::process::exit(0);
    }
    let filename = format!("events-{}-{}.jsonl", session_id, now.format("%Y-%m-%d-%H"));
    let path = dir.join(filename);

    let mut file = match OpenOptions::new().create(true).append(true).open(&path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ccbuddy-hook: failed to open {}: {}", path.display(), e);
            std::process::exit(0);
        }
    };

    if writeln!(file, "{}", entry).is_err() || file.flush().is_err() {
        eprintln!("ccbuddy-hook: failed to write {}", path.display());
        std::process::exit(0);
    }

    // 6. 全局清理：文件数 > MAX_FILES 时删除最旧的，仅保留最新 KEEP_FILES 个
    cleanup(&dir);
}

/// 日志源目录：`~/.claude/data/events`。
fn events_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("data")
        .join("events")
}

/// 收集 `events-*.jsonl` 文件并按文件名排序（文件名含 UTC 时间戳，字典序即时间序）。
fn list_log_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .map(|n| n.to_string_lossy().starts_with("events-"))
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    files
}

fn cleanup(dir: &Path) {
    let files = list_log_files(dir);
    if files.len() <= MAX_FILES {
        return;
    }
    let remove_count = files.len() - KEEP_FILES;
    for f in files.iter().take(remove_count) {
        let _ = fs::remove_file(f);
    }
}
