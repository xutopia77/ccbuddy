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
    pub payload: Value,
}

impl Event {
    /// 解析日志文件中的一行。
    ///
    /// `session_id` 优先取 `payload.session_id`，若缺失则回退到文件名前缀
    /// （`event-<session_id>.jsonl` 中提取）。
    pub fn parse(line: &str, fallback_session: &str) -> Option<Event> {
        let entry: LogEntry = serde_json::from_str(line).ok()?;
        let payload = entry.payload.unwrap_or(Value::Null);

        // hook_event 为空时回退到 payload.hook_event_name（兼容旧版本日志）
        let hook_event = entry
            .hook_event
            .filter(|s| !s.is_empty())
            .or_else(|| {
                payload
                    .get("hook_event_name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
            .unwrap_or_default();

        let session_id = payload
            .get("session_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| fallback_session.to_string());

        let cwd = payload
            .get("cwd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Some(Event {
            received_at: entry.received_at.unwrap_or_default(),
            hook_event,
            session_id,
            cwd,
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
            Some(Value::Object(o)) => ["content", "text", "message"]
                .iter()
                .find_map(|k| o.get(*k).and_then(|v| v.as_str()))
                .map(|s| s.to_string()),
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

    /// 是否携带错误标记。
    pub fn is_error(&self) -> bool {
        self.payload
            .get("is_error")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}
