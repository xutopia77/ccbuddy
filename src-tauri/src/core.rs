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
    log::debug!("RPC 命令: {cmd}");
    let resp = match handle(ctx, cmd, data) {
        Ok(v) => ok(cmd, v),
        Err(e) => {
            log::warn!("命令 {cmd} 失败: code={} status={}", e.code, e.status);
            err(cmd, e.code, e.status)
        }
    };
    log::debug!("RPC 响应: {cmd} code={} data_size={}", resp.code, resp.data.to_string().len());
    resp
}

/// 命令路由表：新增命令在此添加分支。
fn handle(ctx: &RpcContext, cmd: &str, data: Value) -> Result<Value, AppError> {
    match cmd {
        // 会话列表（含 hook 日志实时会话 + 原生历史会话）；懒加载，messages 为空
        "get_sessions" => Ok(json!(crate::state::load_sessions())),
        // 单个会话的完整消息（用户点开详情时按需解析 jsonl）
        "get_session_detail" => {
            let id = data
                .as_str()
                .ok_or_else(|| AppError::bad_request("会话 id 需为字符串"))?;
            crate::state::load_session_detail(id)
                .map(|s| json!(s))
                .ok_or_else(|| AppError::not_found(format!("会话不存在: {id}")))
        }
        // 日志源目录路径
        "get_events_dir" => Ok(json!(crate::state::events_dir().to_string_lossy().to_string())),
        // hook 安装/注册状态（设置页展示）
        "get_hook_status" => Ok(crate::state::hook_status()),
        // 一键安装 hook
        "install_hooks" => {
            let msg = crate::install_hooks_with(ctx.hook_candidates.clone())
                .map_err(AppError::internal)?;
            Ok(json!(msg))
        }
        // 运行时调整日志打印等级
        "set_log_level" => {
            let level_str = data
                .as_str()
                .ok_or_else(|| AppError::bad_request("日志等级需为字符串 (error/warn/info/debug/trace)"))?;
            let level = crate::logger::Level::parse(level_str)
                .ok_or_else(|| AppError::bad_request(format!("无效日志等级: {level_str}")))?;
            crate::logger::set_level(level);
            log::info!("日志等级已设置为 {}", level.as_str());
            Ok(json!(format!("日志等级已设置为 {}", level.as_str())))
        }
        _ => Err(AppError::bad_request(format!("未知命令: {cmd}"))),
    }
}
