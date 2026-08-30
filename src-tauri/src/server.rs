//! 内嵌 HTTP 服务（无头 Linux / 浏览器访问场景）。
//!
//! 提供与桌面端完全一致的 Vue 前端（编译产物在 ccbuddy-server 构建时通过
//! include_dir 嵌入二进制）以及统一的 RPC 接口：
//! - `GET /`          — Vue 前端（SPA）
//! - `POST /api/rpc`  — 统一命令入口（与桌面端 `invoke("rpc")` 走同一套业务分发）
//!
//! ccbuddy-server 可通过命令行参数或 `CCBUDDY_ADDR` 环境变量指定监听地址，
//! 如 `ccbuddy-server 0.0.0.0:8787`（远程服务器供外部浏览器访问）。

use axum::extract::State;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use include_dir::{Dir, File};

use crate::core::{self, RpcContext};
use crate::rpc::{RpcRequest, RpcResponse};

/// 路由共享状态：嵌入的前端静态资源。
#[derive(Clone, Copy)]
struct AppState {
    assets: &'static Dir<'static>,
}

pub async fn start(addr: &str, assets: &'static Dir<'static>) {
    let state = AppState { assets };
    let app = Router::new()
        .route("/api/rpc", post(rpc_api))
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

/// 统一 RPC 入口：与桌面端 `invoke("rpc")` 走同一套业务分发。
async fn rpc_api(Json(req): Json<RpcRequest>) -> Json<RpcResponse> {
    let ctx = server_context();
    Json(core::dispatch(&ctx, &req.cmd, req.data))
}

/// 构造 Server 环境上下文：hook 候选源 = 可执行文件同目录。
fn server_context() -> RpcContext {
    let hook_name = crate::hook_file_name();
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(hook_name));
        }
    }
    RpcContext {
        hook_candidates: candidates,
    }
}

/// 静态资源：嵌入的 Vue 前端（SPA，未知路径回退 index.html）。
async fn static_handler(State(state): State<AppState>, uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let file = if path.is_empty() {
        state.assets.get_file("index.html")
    } else {
        state
            .assets
            .get_file(path)
            .or_else(|| state.assets.get_file("index.html"))
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
