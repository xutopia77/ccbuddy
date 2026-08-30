//! 统一 RPC 协议：前端与后端交互的唯一入口。
//!
//! 前端（WebView）与后端（Rust）之间只通过 `cmd` + `data` 通信，
//! 屏蔽底层 Tauri `invoke` 与 HTTP 的差异。
//!
//! 请求（前端 → 后端）：
//! ```json
//! { "time": "2026-08-30T12:34:56.789Z", "cmd": "get_sessions", "data": null }
//! ```
//!
//! 响应（后端 → 前端，事件推送复用同一结构）：
//! ```json
//! { "time": "2026-08-30T12:34:56.791Z", "cmd": "get_sessions", "code": 0, "status": "ok", "data": [...] }
//! ```

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 成功状态码。
pub const CODE_OK: i32 = 0;
/// 参数错误 / 未知命令。
pub const CODE_BAD_REQUEST: i32 = 400;
/// 资源不存在（预留，当前命令尚未使用）。
#[allow(dead_code)]
pub const CODE_NOT_FOUND: i32 = 404;
/// 内部错误。
pub const CODE_INTERNAL: i32 = 500;

/// 请求：前端主动调用后端命令。
#[derive(Debug, Clone, Deserialize)]
pub struct RpcRequest {
    /// 请求时间（UTC，带毫秒）。协议字段，后端暂不读取。
    #[allow(dead_code)]
    pub time: String,
    /// 命令名。
    pub cmd: String,
    /// 请求数据（可为空）。
    #[serde(default)]
    pub data: Value,
}

/// 响应：后端返回给前端；事件推送也复用该结构（`cmd` 为事件名）。
#[derive(Debug, Clone, Serialize)]
pub struct RpcResponse {
    pub time: String,
    pub cmd: String,
    /// 状态码数字：0 成功，非 0 失败。
    pub code: i32,
    /// 状态描述字符串：成功为 "ok"，失败为错误说明。
    pub status: String,
    /// 返回数据（成功时为业务数据，失败时为 null）。
    pub data: Value,
}

/// 业务错误：业务函数返回它，由命令分发层统一转成 `RpcResponse`。
#[derive(Debug)]
pub struct AppError {
    pub code: i32,
    pub status: String,
}

impl AppError {
    pub fn new(code: i32, status: impl Into<String>) -> Self {
        Self {
            code,
            status: status.into(),
        }
    }

    pub fn bad_request(status: impl Into<String>) -> Self {
        Self::new(CODE_BAD_REQUEST, status)
    }

    #[allow(dead_code)]
    pub fn not_found(status: impl Into<String>) -> Self {
        Self::new(CODE_NOT_FOUND, status)
    }

    pub fn internal(status: impl Into<String>) -> Self {
        Self::new(CODE_INTERNAL, status)
    }
}

/// 构造成功响应。
pub fn ok(cmd: &str, data: Value) -> RpcResponse {
    RpcResponse {
        time: now_ms(),
        cmd: cmd.to_string(),
        code: CODE_OK,
        status: "ok".to_string(),
        data,
    }
}

/// 构造失败响应。
pub fn err(cmd: &str, code: i32, status: impl Into<String>) -> RpcResponse {
    RpcResponse {
        time: now_ms(),
        cmd: cmd.to_string(),
        code,
        status: status.into(),
        data: Value::Null,
    }
}

/// 当前 UTC 时间戳（RFC3339，带毫秒），如 `2026-08-30T12:34:56.789Z`。
pub fn now_ms() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%S%.3fZ").to_string()
}
