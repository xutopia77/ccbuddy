//! 业务命令层：纯 Rust，不依赖 Tauri。
//!
//! 所有前端可调用的命令在此集中注册与分发。适配层（Tauri GUI / HTTP Server）
//! 只需调用 [`dispatch`]，业务新增命令时只改这里与前端 `api.ts`，无需触碰
//! Tauri 或 HTTP 相关代码。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::rpc::{err, ok, AppError, RpcResponse};

/// 命令运行上下文：由各适配层注入的环境信息。
#[derive(Debug, Default)]
pub struct RpcContext {
    /// `ccbuddy-hook` 可执行文件的候选源路径（`install_hooks` 命令用）。
    pub hook_candidates: Vec<PathBuf>,
}

/// 命令分发入口：根据 `cmd` 路由到对应业务函数，统一包装为 `RpcResponse`。
pub fn dispatch(ctx: &RpcContext, cmd: &str, data: Value) -> RpcResponse {
    match handle(ctx, cmd, data) {
        Ok(v) => ok(cmd, v),
        Err(e) => err(cmd, e.code, e.status),
    }
}

/// 命令路由表：新增命令在此添加分支。
fn handle(ctx: &RpcContext, cmd: &str, _data: Value) -> Result<Value, AppError> {
    match cmd {
        // 会话列表（含 hook 日志实时会话 + 原生历史会话）
        "get_sessions" => Ok(json!(crate::state::load_sessions())),
        // 日志源目录路径
        "get_events_dir" => Ok(json!(crate::state::events_dir().to_string_lossy().to_string())),
        // 一键安装 hook
        "install_hooks" => {
            let msg = crate::install_hooks_with(ctx.hook_candidates.clone())
                .map_err(AppError::internal)?;
            Ok(json!(msg))
        }
        _ => Err(AppError::bad_request(format!("未知命令: {cmd}"))),
    }
}
