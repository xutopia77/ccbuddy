//! 内嵌 HTTP 服务（无头 Linux / 浏览器访问场景）。
//!
//! 提供与桌面端完全一致的 Vue 前端（编译产物在 ccbuddy-server 构建时通过
//! include_dir 嵌入二进制）以及 REST API：
//! - `GET /`                — Vue 前端（SPA）
//! - `GET /api/sessions`    — 会话列表 JSON
//! - `GET /api/stats`       — 各状态计数
//! - `GET /api/events_dir`  — 日志源目录
//! - `POST /api/install_hooks` — 安装/更新 hook（与桌面端"一键安装"一致）
//!
//! ccbuddy-server 可通过命令行参数或 `CCBUDDY_ADDR` 环境变量指定监听地址，
//! 如 `ccbuddy-server 0.0.0.0:8787`（远程服务器供外部浏览器访问）。

use std::collections::HashMap;

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use include_dir::{Dir, File};
use serde_json::Value;

use crate::state;

/// 路由共享状态：嵌入的前端静态资源。
#[derive(Clone, Copy)]
struct AppState {
    assets: &'static Dir<'static>,
}

pub async fn start(addr: &str, assets: &'static Dir<'static>) {
    let state = AppState { assets };
    let app = Router::new()
        .route("/api/sessions", get(sessions_api))
        .route("/api/stats", get(stats_api))
        .route("/api/events_dir", get(events_dir_api))
        .route("/api/install_hooks", post(install_hooks_api))
        .fallback(static_handler)
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ccbuddy] 无法监听 {addr}（端口可能被占用）: {e}");
            return;
        }
    };
    println!("[ccbuddy] HTTP 服务已启动: http://{addr}");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[ccbuddy] HTTP 服务异常: {e}");
    }
}

async fn sessions_api() -> Json<Vec<state::SessionInfo>> {
    Json(state::load_sessions())
}

async fn stats_api() -> Json<Value> {
    let sessions = state::load_sessions();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in &sessions {
        *counts.entry(s.status.clone()).or_insert(0) += 1;
    }
    counts.insert("total".to_string(), sessions.len());
    Json(serde_json::to_value(counts).unwrap_or(Value::Null))
}

async fn events_dir_api() -> String {
    state::events_dir().to_string_lossy().to_string()
}

/// 与桌面端"一键安装"相同：从 server 可执行文件同目录定位 hook。
async fn install_hooks_api() -> Result<String, Response> {
    let hook_name = if cfg!(windows) {
        "ccbuddy-hook.exe"
    } else {
        "ccbuddy-hook"
    };
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(hook_name));
        }
    }
    crate::install_hooks_with(candidates).map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, e).into_response())
}

/// 静态资源：嵌入的 Vue 前端（SPA，未知路径回退 index.html）。
async fn static_handler(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = if path.is_empty() {
        state.assets.get_file("index.html")
    } else {
        state.assets.get_file(path).or_else(|| state.assets.get_file("index.html"))
    };
    match file {
        Some(f) => file_response(f),
        None => (StatusCode::NOT_FOUND, "404 Not Found").into_response(),
    }
}

/// 按扩展名推断 Content-Type 并返回文件内容。
fn file_response(file: &'static File<'static>) -> Response {
    let mime = mime_of(file.path().to_str().unwrap_or(""));
    ([(header::CONTENT_TYPE, mime)], file.contents()).into_response()
}

fn mime_of(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" | "htm" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript",
        "css" => "text/css",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "ico" => "image/x-icon",
        "json" => "application/json",
        "woff" => "font/woff",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}
