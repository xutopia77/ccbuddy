//! 内嵌 HTTP 服务（无头 Linux / 浏览器访问场景）。
//!
//! 提供与桌面端完全一致的 Vue 前端（编译产物在 ccbuddy-server 构建时通过
//! include_dir 嵌入二进制）以及统一的 RPC 接口：
//! - `GET /`          — Vue 前端（SPA）
//! - `POST /api/rpc`  — 统一命令入口（与桌面端 `invoke("rpc")` 走同一套业务分发）
//!
//! ccbuddy-server 可通过命令行参数或 `CCBUDDY_ADDR` 环境变量指定监听地址，
//! 如 `ccbuddy-server 0.0.0.0:18787`（远程服务器供外部浏览器访问）。
//!
//! ## 访问验证（多用户主机防窥探）
//! 应用自己的登录页 + session token，**不用** Basic Auth（那会弹浏览器原生弹框，
//! 样式不可控、凭据也无过期语义）：
//! - 静态资源公开（否则登录页本身都加载不出来），`/api/*` 除登录相关外一律 401
//! - `POST /api/login` 校验密码后签发 token，以 HttpOnly cookie 下发。
//!   用 cookie 而非 `Authorization` 头：`EventSource` 无法设置请求头，
//!   cookie 是唯一能让 `fetch` 与 SSE 长连接共用一套凭证的机制
//! - 空闲 15 分钟 / 签发满 15 天失效；登录态只存内存，重启即全部重新登录
//! - 首次启动无密码配置时自动生成 16 位随机密码，哈希落盘、明文仅终端打印一次
//!
//! 会话表与密码校验在 [`crate::auth`]。

use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode, Uri};
use axum::response::sse::{Event as SseEvent, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use futures_util::stream::Stream;
use include_dir::{Dir, File};

use crate::auth;
use crate::core::{self, RpcContext};
use crate::proto::{RpcRequest, RpcResponse};

/// 路由共享状态：嵌入的前端静态资源。
#[derive(Clone, Copy)]
struct AppState {
    assets: &'static Dir<'static>,
}

/// SSE 推送通道：watcher 检测到事件流变化 → 广播 RpcResponse 信封。
/// tokio broadcast 是 MPMC：每个 SSE 连接持有一个 receiver，互不抢消息。
fn sse_channel() -> &'static tokio::sync::broadcast::Sender<crate::proto::RpcResponse> {
    static TX: std::sync::OnceLock<tokio::sync::broadcast::Sender<crate::proto::RpcResponse>> =
        std::sync::OnceLock::new();
    TX.get_or_init(|| {
        let (tx, _rx) = tokio::sync::broadcast::channel(32);
        // watcher 是同步线程回调，转到 tokio 上下文发送。
        // guard 故意 forget：与 server 同生命周期（静态 channel 常驻，监听不注销）
        let tx2 = tx.clone();
        let guard = crate::events_mgr::subscribe(Box::new(move |sessions| {
            let resp = crate::proto::ok("events_changed", serde_json::json!(sessions));
            let _ = tx2.send(resp);
        }));
        std::mem::forget(guard);
        // 用量变更推送：与事件流同通道（信封 cmd 区分，前端按 cmd 分发）。
        // guard 同上 forget（常驻）。
        let tx3 = tx.clone();
        let guard2 = crate::usage_mgr::subscribe_usage(Box::new(move |records| {
            let resp = crate::proto::ok("usage_changed", serde_json::json!(records));
            let _ = tx3.send(resp);
        }));
        std::mem::forget(guard2);
        tx
    })
}

pub async fn start(addr: &str, assets: &'static Dir<'static>) {
    let state = AppState { assets };

    // 需要登录的接口单独挂中间件；登录相关接口与静态资源公开。
    // 用 route_layer 而非 layer：它**不作用于 fallback**，静态资源因此顺带公开
    // （登录页得先能加载出来），这正是这里想要的效果，不是巧合。
    let protected = Router::new()
        .route("/api/rpc", post(rpc_api))
        .route("/api/events", get(sse_events))
        .route_layer(axum::middleware::from_fn(session_auth));

    let app = Router::new()
        .route("/api/login", post(login))
        .route("/api/logout", post(logout))
        .route("/api/session", get(session))
        // `/api/*` 下未注册的路径直接 404，别落到 index.html 回退（否则返回 200 HTML）
        .route("/api/{*rest}", any(api_not_found))
        .merge(protected)
        // fallback 只能在最终 router 上设：合并的两个 router 都有 fallback 会 panic
        .fallback(static_handler)
        .with_state(state);

    // 无密码配置（首次启动，或清空密码字段后重启）：生成随机密码并哈希落盘，
    // 明文只在这里打印一次。返回的 (is_hash, value) 已无用——登录校验每次现读配置
    // （见 auth::check_password）。
    if let (_, Some(plain)) = crate::config::server_password_or_init() {
        println!("==============================================================");
        println!("未设置访问密码，已生成随机密码: {plain}");
        println!("请记录后登录，并在「设置 → 修改密码」中改为自己的密码");
        println!("==============================================================");
        log::info!("生成随机访问密码（仅终端显示一次）");
    }
    log::info!("已启用访问验证（登录页 + session token，空闲 15 分钟 / 最长 15 天）");

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("无法监听 {addr}（端口可能被占用）: {e}");
            return;
        }
    };
    // 启动事件流 watcher（SSE 推送源；幂等，重复调用无副作用）
    let sse_tx = sse_channel();
    let _ = sse_tx;
    log::info!("HTTP 服务已启动: http://{addr}");
    if let Err(e) = axum::serve(listener, app).await {
        log::error!("HTTP 服务异常: {e}");
    }
}

// ---- 登录 / 登出 / 登录态 ----

/// 登录请求体。
///
/// 只有密码：用户名固定 `admin`，但**沿用「用户名任意、只验密码」的宽容语义**
/// （见 [`crate::auth::check_password`]）——加一个固定值分支既无安全收益，
/// 又会用响应时序泄露用户名。
#[derive(serde::Deserialize)]
struct LoginBody {
    #[serde(default)]
    password: String,
}

/// `POST /api/login`：校验密码，成功则签发 session 并下发 cookie。
async fn login(Json(body): Json<LoginBody>) -> Response {
    // Argon2 校验是 CPU 密集运算（~100ms 防爆破成本），放进阻塞线程池
    let pwd = body.password;
    let ok = tokio::task::spawn_blocking(move || auth::check_password(&pwd))
        .await
        .unwrap_or(false);
    if !ok {
        return no_store((StatusCode::UNAUTHORIZED, "401 Unauthorized").into_response());
    }

    let token = auth::sessions().issue();
    let cookie = match HeaderValue::from_str(&cookie_value(&token)) {
        Ok(v) => v,
        // token 是纯 hex、模板是固定 ASCII，理论不可达。真出错时宁可拒绝登录，
        // 也不要下发一个畸形 cookie 让用户对着「登录成功但仍未登录」排查。
        Err(_) => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "cookie 构造失败").into_response();
        }
    };
    let mut resp = Json(serde_json::json!({ "ok": true })).into_response();
    resp.headers_mut().insert(header::SET_COOKIE, cookie);
    no_store(resp)
}

/// `POST /api/logout`：撤销当前 token 并清 cookie。
///
/// 幂等：没带 cookie 或 token 早已失效也返回成功。
async fn logout(headers: HeaderMap) -> Response {
    if let Some(token) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()).and_then(cookie_token) {
        auth::sessions().revoke(&token);
    }
    let mut resp = Json(serde_json::json!({ "ok": true })).into_response();
    resp.headers_mut()
        .insert(header::SET_COOKIE, HeaderValue::from_static(CLEAR_COOKIE));
    no_store(resp)
}

/// `GET /api/session`：当前登录态，前端用于启动探测 + 可见时心跳（同一个问题，同一个端点）。
///
/// 返回 200 + `{authed}` 而不是 401：调用方是在问「我现在是什么状态」，
/// 不是「请给我数据」，用状态码表达反而要为正常回答走异常分支。
///
/// 它同时是**唯一**能发现「服务重启导致 token 全部失效」的途径——SSE 断线不给状态码。
async fn session(headers: HeaderMap) -> Response {
    let authed = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(cookie_token)
        .map(|t| auth::sessions().validate(&t))
        .unwrap_or(false);
    no_store(Json(serde_json::json!({ "authed": authed })).into_response())
}

/// session 校验中间件：只认本服务签发的 token（由 cookie 携带）。
///
/// 校验通过即续期（这是一次真实的用户请求）。
async fn session_auth(req: axum::extract::Request, next: axum::middleware::Next) -> Response {
    let ok = req
        .headers()
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(cookie_token)
        .map(|t| auth::sessions().validate(&t))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        unauthorized()
    }
}

/// 401 响应。
///
/// **刻意不带 `WWW-Authenticate`**：带了浏览器就会弹自己的 Basic Auth 弹框，
/// 而这里要的是前端显示应用自己的登录页。
fn unauthorized() -> Response {
    no_store((StatusCode::UNAUTHORIZED, "401 Unauthorized").into_response())
}

/// `Set-Cookie` 值：HttpOnly（JS 读不到）、SameSite=Strict（跨站不携带）、
/// `Path=/`、`Max-Age` 与 [`auth::ABSOLUTE_TIMEOUT`] 对齐。
///
/// 不设 `Max-Age` 就是会话 cookie，关浏览器即失效，与「15 天强制过期」自相矛盾，
/// 所以必须显式设。**不加 `Secure`**：本服务不提供 TLS，加了 cookie 直接发不出去
/// （代价是明文 HTTP 下 cookie 与密码都走明文，与改造前的 Basic Auth 同等）。
fn cookie_value(token: &str) -> String {
    format!(
        "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={}",
        auth::COOKIE_NAME,
        auth::ABSOLUTE_TIMEOUT.as_secs(),
    )
}

/// 清除 cookie（登出）。`Max-Age=0` 让浏览器立即丢弃。
const CLEAR_COOKIE: &str = "ccbuddy_session=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0";

/// 从 `Cookie` 头里取本服务的 session token。
///
/// 手写最小解析（按 `;` 分段、找 `名字=`），不引 cookie crate——全程只有这一个 cookie。
fn cookie_token(header: &str) -> Option<String> {
    header.split(';').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k.trim() == auth::COOKIE_NAME).then(|| v.trim().to_string())
    })
}

/// 认证相关响应统一加 `Cache-Control: no-store`。
/// 否则登出后浏览器回退可能读到缓存的 `{"authed":true}` 或登录响应。
fn no_store(mut resp: Response) -> Response {
    resp.headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    resp
}

/// `/api/*` 下未注册路径的 404（避免落到 `static_handler` 的 index.html 回退）。
async fn api_not_found() -> Response {
    (StatusCode::NOT_FOUND, "404 Not Found").into_response()
}

// ---- RPC 与推送 ----

/// 统一 RPC 入口：与桌面端 `invoke("rpc")` 走同一套业务分发。
async fn rpc_api(Json(req): Json<RpcRequest>) -> Json<RpcResponse> {
    let ctx = server_context();
    Json(core::dispatch(&ctx, &req.cmd, req.data))
}

/// SSE 推送端点（`GET /api/events`）：事件流变化时推送 RpcResponse 信封。
///
/// 连接建立时先推一次全量列表（等价轮询的首次 load），之后只在有变化时推。
/// 信封格式与 `/api/rpc` 响应一致：`{ time, cmd, code, status, data }`，
/// data 为会话列表（与 get_events 命令的返回完全相同）。
async fn sse_events(headers: HeaderMap) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let token = headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(cookie_token)
        .unwrap_or_default();

    let tx = sse_channel();
    let rx = tx.subscribe();

    // 首次推送：当前全量列表（连接建立即有数据，前端不必再发一次 get_events）
    let initial = crate::proto::ok("events_changed", serde_json::json!(crate::events_mgr::load_events()));

    let stream = futures_util::stream::unfold(
        (rx, Some(initial), token),
        |(mut rx, initial, token)| async move {
            // 长连接只在握手时过了一次中间件，这里每次产出前复查：
            // 登出 / 改密码 / 空闲过期后立即断流。
            // 用 is_valid（**不续期**）——SSE 长连接存活不算「用户活跃」，
            // 否则后台标签页会被持续推送一直续命，15 分钟空闲过期就形同虚设了。
            // 残留：系统完全安静时流里没有 item，本检查不执行；由前端心跳兜底（≤60 秒）。
            if !auth::sessions().is_valid(&token) {
                return None;
            }
            // initial 只在第一次产出，之后全部走 broadcast
            if let Some(first) = initial {
                let item = sse_frame(&first);
                return Some((Ok(item), (rx, None, token)));
            }
            match rx.recv().await {
                Ok(resp) => Some((Ok(sse_frame(&resp)), (rx, None, token))),
                // Lagged（错过消息，容量 16 溢出）：跳过并继续，下一轮指纹变化会补上
                Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                    log::warn!("SSE 推送滞后，跳过 {n} 条");
                    // 递归继续等下一条；用 broadcast 的清空语义简单处理
                    match rx.recv().await {
                        Ok(resp) => Some((Ok(sse_frame(&resp)), (rx, None, token))),
                        Err(_) => None,
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
            }
        },
    );

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(std::time::Duration::from_secs(15))
            .text("keep-alive"),
    )
}

/// 把 RpcResponse 包装成 SSE 帧：`event: events_changed` + JSON data。
fn sse_frame(resp: &RpcResponse) -> SseEvent {
    SseEvent::default()
        .event(resp.cmd.as_str())
        .data(serde_json::to_string(resp).unwrap_or_default())
}

/// 构造 Server 环境上下文：hook 候选源 = 可执行文件同目录。
fn server_context() -> RpcContext {
    let mut candidates: Vec<std::path::PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            for name in crate::hook_candidate_names() {
                candidates.push(dir.join(name));
            }
        }
    }
    RpcContext {
        hook_candidates: candidates,
    }
}

// ---- 静态资源 ----

/// 静态资源：嵌入的 Vue 前端（SPA，未知路径回退 index.html）。
///
/// 公开访问：SPA 包内无任何秘密（与桌面端发布的是同一份产物），
/// 且登录页本身就得先加载出来才谈得上登录。
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_value_carries_flags_and_absolute_max_age() {
        let v = cookie_value("abc123");
        assert!(v.starts_with("ccbuddy_session=abc123;"));
        assert!(v.contains("HttpOnly"));
        assert!(v.contains("SameSite=Strict"));
        assert!(v.contains("Path=/"));
        // Max-Age 必须与绝对过期一致，否则 cookie 先于服务端判断失效
        assert!(v.contains(&format!("Max-Age={}", 15 * 24 * 60 * 60)));
        // 没有 TLS，加了 Secure 就发不出去
        assert!(!v.contains("Secure"));
    }

    #[test]
    fn cookie_token_extracts_only_our_cookie() {
        assert_eq!(cookie_token("ccbuddy_session=deadbeef"), Some("deadbeef".to_string()));
        // 多个 cookie：只挑自己的
        assert_eq!(
            cookie_token("a=1; ccbuddy_session=deadbeef; b=2"),
            Some("deadbeef".to_string())
        );
        // 空白容错
        assert_eq!(cookie_token("  ccbuddy_session = deadbeef "), Some("deadbeef".to_string()));
        // 别人的 cookie / 空串 / 畸形：一律 None
        assert_eq!(cookie_token("other=deadbeef"), None);
        assert_eq!(cookie_token(""), None);
        assert_eq!(cookie_token("ccbuddy_session"), None);
        // 名字必须整段匹配，不能被前缀骗过
        assert_eq!(cookie_token("ccbuddy_session_old=deadbeef"), None);
    }

    #[test]
    fn clear_cookie_expires_immediately() {
        assert!(CLEAR_COOKIE.contains("ccbuddy_session=;"));
        assert!(CLEAR_COOKIE.contains("Max-Age=0"));
    }
}
