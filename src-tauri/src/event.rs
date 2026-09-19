//! hook 事件模型：一行日志 JSON → 内存中的统一事件。

use serde::Deserialize;
use serde_json::Value;

/// 日志文件中的一行（Hook Logger 写入的包装结构）：
/// ```json
/// { "received_at": "...", "hook_event": "PreToolUse", "payload": { ... } }
/// ```
#[derive(Debug, Clone, Deserialize)]
pub struct LogEntry {
    #[serde(default)]
    pub received_at: Option<String>,
    #[serde(default)]
    pub hook_event: Option<String>,
    #[serde(default)]
    pub payload: Option<Value>,
}

/// 解析后的统一事件模型（仅在内存中使用，不持久化）。
#[derive(Debug, Clone)]
pub struct Event {
    pub received_at: String,
    pub hook_event: String,
    pub session_id: String,
    pub cwd: Option<String>,
    /// 子代理事件标记：非 None 表示该事件由子代理（Task/Agent 工具）触发，
    /// 与主会话共用 session_id，但不参与主会话状态机（见 events_mgr）。
    pub parent_tool_use_id: Option<String>,
    /// transcript 路径（多数事件带）：会话原始记录文件位置。
    pub transcript_path: Option<String>,
    /// SessionStart 触发来源：startup / resume / clear / compact / fork。
    pub source: Option<String>,
    /// SessionEnd 结束原因：clear / resume / logout / prompt_input_exit / other。
    pub reason: Option<String>,
    pub payload: Value,
}

impl Event {
    /// 解析日志文件中的一行。
    ///
    /// `session_id` 优先取 `payload.session_id`，若缺失则回退到文件名前缀
    /// （`event-<session_id>.jsonl` 中提取）。
    pub fn parse(line: &str, fallback_session: &str) -> Option<Event> {
        // 非 JSON 行返回 None，由调用方跳过
        let entry: LogEntry = serde_json::from_str(line).ok()?;
        let payload = entry.payload.unwrap_or(Value::Null);

        // hook_event 为空时回退到 payload.hook_event_name（兼容旧版本日志）
        let hook_event: String = match entry.hook_event.filter(|s| !s.is_empty()) {
            Some(s) => s,
            None => payload
                .get("hook_event_name")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string(),
        };

        // session_id 优先取 payload 中的值，缺失时回退到文件名提取的 id
        let session_id: String = match payload.get("session_id").and_then(|v| v.as_str()) {
            Some(s) => s.to_string(),
            None => fallback_session.to_string(),
        };

        let cwd: Option<String> = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        // 顶层可选字段：payload 顶层（非嵌套）取字符串
        let top_str = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
        };

        Some(Event {
            received_at: entry.received_at.unwrap_or_default(),
            hook_event,
            session_id,
            cwd,
            parent_tool_use_id: top_str("parent_tool_use_id"),
            transcript_path: top_str("transcript_path"),
            source: top_str("source"),
            reason: top_str("reason"),
            payload,
        })
    }

    /// 从 payload 中提取工具名（PreToolUse/PostToolUse 事件）。
    pub fn tool_name(&self) -> Option<String> {
        self.payload
            .get("tool_name")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    /// 从 payload 中提取消息内容（兼容 message 为字符串或对象）。
    pub fn message(&self) -> Option<String> {
        match self.payload.get("message") {
            Some(Value::String(s)) => Some(s.clone()),
            Some(Value::Object(o)) => {
                // 依次尝试 content / text / message 三个字段，取第一个字符串
                for key in ["content", "text", "message"] {
                    if let Some(v) = o.get(key).and_then(|v| v.as_str()) {
                        return Some(v.to_string());
                    }
                }
                None
            }
            _ => None,
        }
    }

    /// 从 payload 中提取用户提示词（UserPromptSubmit 事件）。
    pub fn prompt(&self) -> Option<String> {
        self.payload
            .get("prompt")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }
}
