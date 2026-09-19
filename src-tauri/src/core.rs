//! 业务命令层：纯 Rust，不依赖 Tauri。
//!
//! 所有前端可调用的命令在此集中注册与分发。适配层（Tauri GUI / HTTP Server）
//! 只需调用 [`dispatch`]，业务新增命令时只改这里与前端 `api.ts`，无需触碰
//! Tauri 或 HTTP 相关代码。

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::proto::{err, ok, AppError, RpcResponse};

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
        // 事件流会话列表（hook 日志，增量刷新，每会话最新50条事件）
        "get_events" => Ok(json!(crate::events_mgr::load_events())),
        // 会话列表（Claude Code 原生 transcript，与事件流分开）
        "get_sessions" => Ok(json!(crate::claude::load_sessions())),
        // L-C8：Claude Code 官方 transcript 保留期（cleanupPeriodDays，只读；
        // 未配置返回 null，前端不显示提示）
        "get_transcript_retention" => Ok(json!(crate::claude::transcript_retention_days())),
        // 全部 API 请求的 token 用量记录（原生 transcript，mtime 增量缓存，时间倒序）
        "get_usage" => Ok(json!(crate::usage_mgr::load_usage())),
        // 事件流会话详情（hook 日志，用户点开时解析该会话最新50条事件）
        "get_event_detail" => {
            let id = data
                .as_str()
                .ok_or_else(|| AppError::bad_request("会话 id 需为字符串"))?;
            crate::events_mgr::load_event_detail(id)
                .map(|s| json!(s))
                .ok_or_else(|| AppError::not_found(format!("会话不存在: {id}")))
        }
        // 历史会话详情（原生 transcript，用户点开时解析全量消息）
        "get_session_detail" => {
            let id = data
                .as_str()
                .ok_or_else(|| AppError::bad_request("会话 id 需为字符串"))?;
            crate::claude::load_session_detail(id)
                .map(|s| json!(s))
                .ok_or_else(|| AppError::not_found(format!("会话不存在: {id}")))
        }
        // 读取用户配置（含只读的 events_dir 等派生字段）
        "get_config" => Ok(json!(crate::config::config_view())),
        // 更新用户配置（部分更新：只覆盖传入的字段；log_level 为运行时项不入盘）
        "set_config" => {
            let patch = data
                .as_object()
                .ok_or_else(|| AppError::bad_request("配置需为对象"))?;
            crate::config::apply_patch(patch).map_err(AppError::internal)?;
            Ok(json!(crate::config::config_view()))
        }
        // hook 安装/注册状态（设置页展示）
        "get_hook_status" => Ok(crate::events_mgr::hook_status()),
        // 一键安装 hook
        "install_hooks" => {
            let msg = crate::install_hooks_with(ctx.hook_candidates.clone())
                .map_err(AppError::internal)?;
            Ok(json!(msg))
        }
        // 卸载 hook（移除 ccbuddy 注册并删除已安装的 hook 文件，保留用户其他配置）
        "uninstall_hooks" => {
            let msg = crate::uninstall_hooks_with().map_err(AppError::internal)?;
            Ok(json!(msg))
        }
        // 修改 server 访问密码（桌面端界面不提供入口，但命令本身通用）
        "change_password" => {
            let obj = data
                .as_object()
                .ok_or_else(|| AppError::bad_request("密码参数需为对象"))?;
            let old = str_field(obj, "old")?;
            let new = str_field(obj, "new")?;
            change_password(&old, &new)?;
            Ok(json!("密码已修改"))
        }
        _ => Err(AppError::bad_request(format!("未知命令: {cmd}"))),
    }
}

/// 取 RPC 参数中的字符串字段。
fn str_field(obj: &serde_json::Map<String, Value>, key: &str) -> Result<String, AppError> {
    obj.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| AppError::bad_request(format!("缺少字段: {key}")))
}

/// 新密码最短长度。不设上限：Argon2 不像 bcrypt 有 72 字节截断，长密码无害。
const MIN_PASSWORD_LEN: usize = 6;

/// 修改访问密码：验旧 → 哈希新 → 落盘 → 踢掉全部会话。
fn change_password(old: &str, new: &str) -> Result<(), AppError> {
    // 环境变量优先于配置文件（见 auth::env_password），此时写盘的新密码永远读不到，
    // 改动静默失效、用户反被关在门外。诚实拒绝，指路环境变量。
    if crate::auth::env_password().is_some() {
        return Err(AppError::bad_request(
            "当前密码由 CCBUDDY_PASSWORD 环境变量指定，请修改环境变量后重启服务",
        ));
    }
    if new.len() < MIN_PASSWORD_LEN {
        return Err(AppError::bad_request(format!(
            "新密码至少 {MIN_PASSWORD_LEN} 位"
        )));
    }
    if !crate::auth::check_password(old) {
        return Err(AppError::bad_request("原密码错误"));
    }
    let hashed = crate::config::hash_password(new).map_err(AppError::internal)?;
    crate::config::write_server_password(Some(&hashed)).map_err(AppError::internal)?;
    // 全部会话失效：改密码的意义就是让旧凭据立刻作废，否则只挡得住新登录
    crate::auth::sessions().revoke_all();
    log::info!("访问密码已修改，全部会话已失效");
    Ok(())
}
