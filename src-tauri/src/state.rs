use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::event::Event;

/// 会话状态（与前端 App.vue 的状态枚举保持一致）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SessionStatus {
    Running,
    WaitingConfirmation,
    WaitingInput,
    Error,
    Completed,
    Idle,
}

impl SessionStatus {
    fn as_str(&self) -> &'static str {
        match self {
            SessionStatus::Running => "running",
            SessionStatus::WaitingConfirmation => "waiting_confirmation",
            SessionStatus::WaitingInput => "waiting_input",
            SessionStatus::Error => "error",
            SessionStatus::Completed => "completed",
            SessionStatus::Idle => "idle",
        }
    }
}

/// 前端展示用的单条消息。
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    /// "user" | "assistant" | "system"
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    pub role: &'static str,
    pub content: String,
    pub time: String,
    #[serde(rename = "toolCall", skip_serializing_if = "Option::is_none")]
    pub tool_call: Option<String>,
}

/// 前端展示用的会话信息（对应 App.vue 的 Session）。
#[derive(Debug, Clone, Serialize)]
pub struct SessionInfo {
    pub id: String,
    /// 项目名（目录名，用于分组）
    pub project: String,
    /// 项目完整路径
    pub cwd: String,
    pub title: String,
    pub status: String,
    #[serde(rename = "lastActivity")]
    pub last_activity: String,
    pub preview: String,
    pub unread: bool,
    pub messages: Vec<Message>,
}

/// 内存中的会话聚合状态。
struct SessionAgg {
    id: String,
    status: SessionStatus,
    title: String,
    project: String,
    cwd: String,
    last_activity: String,
    preview: String,
    messages: Vec<Message>,
    has_error: bool,
}

impl SessionAgg {
    fn new(id: String) -> Self {
        Self {
            id,
            status: SessionStatus::Idle,
            title: String::new(),
            project: String::new(),
            cwd: String::new(),
            last_activity: String::new(),
            preview: String::new(),
            messages: Vec::new(),
            has_error: false,
        }
    }

    fn set_project_from_cwd(&mut self, cwd: &str) {
        if !cwd.is_empty() {
            let name = cwd
                .trim_end_matches(['/', '\\'])
                .rsplit(['/', '\\'])
                .next()
                .unwrap_or(cwd);
            self.project = name.to_string();
        }
    }

    /// 根据一个事件推进状态机。
    fn apply(&mut self, ev: &Event) {
        self.last_activity = ev.received_at.clone();

        if let Some(cwd) = &ev.cwd {
            self.cwd = cwd.clone();
            self.set_project_from_cwd(cwd);
        }

        match ev.hook_event.as_str() {
            "SessionStart" => {
                self.status = SessionStatus::Idle;
            }
            "UserPromptSubmit" => {
                if let Some(prompt) = ev.prompt() {
                    if !is_system_marker(&prompt) {
                        if self.title.is_empty() {
                            self.title = truncate(&prompt, 30);
                        }
                        self.preview = truncate(&prompt, 60);
                        self.messages.push(Message {
                            msg_type: "user",
                            role: "user",
                            content: prompt,
                            time: short_time(&ev.received_at),
                            tool_call: None,
                        });
                    }
                }
                if self.status != SessionStatus::Error {
                    self.status = SessionStatus::Running;
                }
            }
            "PreToolUse" => {
                let tool = ev.tool_name().unwrap_or_else(|| "工具".to_string());
                let brief = format!("调用工具 {tool}…");
                self.preview = brief.clone();
                self.messages.push(Message {
                    msg_type: "assistant",
                    role: "assistant",
                    content: brief,
                    time: short_time(&ev.received_at),
                    tool_call: Some(tool),
                });
                self.status = SessionStatus::WaitingConfirmation;
            }
            "PostToolUse" => {
                if ev.is_error() {
                    self.status = SessionStatus::Error;
                    self.has_error = true;
                    self.messages.push(system_message("工具调用失败", &ev.received_at));
                } else if self.status != SessionStatus::Error {
                    self.status = SessionStatus::Running;
                }
            }
            "Notification" => {
                if let Some(msg) = ev.message() {
                    if needs_input(&msg) {
                        self.status = SessionStatus::WaitingInput;
                        self.preview = truncate(&msg, 60);
                    } else if ev.is_error() {
                        self.status = SessionStatus::Error;
                        self.has_error = true;
                        self.messages.push(system_message(&msg, &ev.received_at));
                    }
                }
            }
            "Stop" | "SubagentStop" => {
                if ev.is_error() {
                    self.status = SessionStatus::Error;
                    self.has_error = true;
                }
            }
            "SessionEnd" => {
                if !self.has_error {
                    self.status = SessionStatus::Completed;
                }
            }
            // 其余事件（AssistantMessage 等）：若带文本内容则作为 assistant 消息展示。
            _ => {
                if let Some(content) = ev.message() {
                    self.preview = truncate(&content, 60);
                    self.messages.push(Message {
                        msg_type: "assistant",
                        role: "assistant",
                        content,
                        time: short_time(&ev.received_at),
                        tool_call: None,
                    });
                    if self.status != SessionStatus::WaitingConfirmation
                        && self.status != SessionStatus::WaitingInput
                        && self.status != SessionStatus::Error
                    {
                        self.status = SessionStatus::Running;
                    }
                }
            }
        }
    }

    fn into_info(self) -> SessionInfo {
        let unread = matches!(
            self.status,
            SessionStatus::WaitingConfirmation
                | SessionStatus::WaitingInput
                | SessionStatus::Error
        );
        let title = if self.title.is_empty() {
            "(未命名会话)".to_string()
        } else {
            self.title
        };
        let project = if self.project.is_empty() {
            "unknown".to_string()
        } else {
            self.project
        };
        SessionInfo {
            id: self.id,
            project,
            cwd: self.cwd,
            title,
            status: self.status.as_str().to_string(),
            last_activity: self.last_activity,
            preview: self.preview,
            unread,
            messages: self.messages,
        }
    }
}

fn system_message(content: &str, time: &str) -> Message {
    Message {
        msg_type: "system",
        role: "system",
        content: content.to_string(),
        time: short_time(time),
        tool_call: None,
    }
}

/// 通知消息是否要求用户输入（简单启发式）。
fn needs_input(msg: &str) -> bool {
    let lower = msg.to_lowercase();
    ["需要", "输入", "提供", "回复", "请", "确认一下", "等待", "回复"]
        .iter()
        .any(|k| lower.contains(k))
        || lower.contains("need")
        || lower.contains("input")
}

/// 截断字符串到指定字符数（按字符而非字节）。
fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let t: String = s.chars().take(max_chars).collect();
        format!("{t}…")
    }
}

/// 判断文本是否是 Claude Code 系统注入标记（非真实用户输入）。
fn is_system_marker(s: &str) -> bool {
    let t = s.trim_start();
    if !t.starts_with('<') {
        return false;
    }
    [
        "<task-",
        "<local-command-",
        "<command-",
        "<system-",
        "<output-",
        "<session-",
        "<bash-",
        "<tool-",
        "<result-",
        "<overview-",
        "<rewrite-",
        "<progress-",
        "<summary-",
        "<file-",
        "<thinking",
    ]
    .iter()
    .any(|p| t.starts_with(p))
}

/// 从 ISO 时间戳提取 HH:MM 短格式。
fn short_time(iso: &str) -> String {
    // received_at 形如 "2026-08-20T14:35:22.123Z"，取 "T" 后的前 5 个字符。
    iso.split('T')
        .nth(1)
        .map(|t| t.chars().take(5).collect())
        .unwrap_or_else(|| iso.to_string())
}

/// 日志源目录：`~/.ccbuddy/events`（设计文档约定，不提供配置项）。
pub fn events_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".ccbuddy")
        .join("events")
}

/// 从文件名 `event-<session_id>.jsonl` 提取 session_id。
fn session_id_from_filename(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default()
        .strip_prefix("event-")
        .unwrap_or_default()
        .trim_end_matches(".jsonl")
        .to_string()
}

/// 扫描默认日志目录 + 原生历史会话目录，聚合为会话列表。
pub fn load_sessions() -> Vec<SessionInfo> {
    let mut map: HashMap<String, SessionInfo> = HashMap::new();

    // 1. 实时 hook 日志（活跃会话，状态由状态机推断，优先级更高）
    for s in load_sessions_from(&events_dir()) {
        map.insert(s.id.clone(), s);
    }

    // 2. 原生历史会话（Claude Code 过去的会话记录，补足历史视图）
    for s in load_native_sessions() {
        map.entry(s.id.clone()).or_insert(s);
    }

    let mut out: Vec<SessionInfo> = map.into_values().collect();
    out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    out
}

/// 扫描指定目录，解析所有事件，聚合为会话列表。
fn load_sessions_from(dir: &Path) -> Vec<SessionInfo> {
    let mut files: Vec<PathBuf> = match std::fs::read_dir(dir) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.file_name()
                    .map(|n| n.to_string_lossy().starts_with("event-"))
                    .unwrap_or(false)
            })
            .collect(),
        Err(_) => return Vec::new(),
    };
    files.sort();

    let mut sessions: HashMap<String, SessionAgg> = HashMap::new();

    for path in &files {
        let fallback = session_id_from_filename(path);
        let content = std::fs::read_to_string(path).unwrap_or_default();
        for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
            if let Some(ev) = Event::parse(line, &fallback) {
                let agg = sessions
                    .entry(ev.session_id.clone())
                    .or_insert_with(|| SessionAgg::new(ev.session_id.clone()));
                agg.apply(&ev);
            }
        }
    }

    sessions.into_values().map(|a| a.into_info()).collect()
}

/// Claude Code 原生会话目录：`~/.claude/projects/<项目路径编码>/<session-id>.jsonl`。
fn projects_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".claude")
        .join("projects")
}

/// 扫描原生历史会话，聚合为会话列表（历史会话统一标记为 completed）。
fn load_native_sessions() -> Vec<SessionInfo> {
    let dir = projects_dir();
    let mut result = Vec::new();
    let Ok(project_dirs) = std::fs::read_dir(&dir) else {
        return result;
    };

    for project_dir in project_dirs.flatten() {
        let path = project_dir.path();
        if !path.is_dir() {
            continue;
        }
        let Ok(session_files) = std::fs::read_dir(&path) else {
            continue;
        };
        for f in session_files.flatten() {
            let fp = f.path();
            if fp.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }
            if let Some(info) = parse_native_session(&fp) {
                result.push(info);
            }
        }
    }
    result
}

/// 解析单个原生会话文件（Claude Code 的 transcript .jsonl）。
fn parse_native_session(path: &Path) -> Option<SessionInfo> {
    let session_id = path.file_stem()?.to_string_lossy().to_string();
    let content = std::fs::read_to_string(path).ok()?;

    let mut custom_title = String::new();
    let mut ai_title = String::new();
    let mut title = String::new();
    let mut project = String::new();
    let mut cwd = String::new();
    let mut last_activity = String::new();
    let mut preview = String::new();
    let mut messages: Vec<Message> = Vec::new();

    for line in content.lines().map(str::trim).filter(|l| !l.is_empty()) {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let Some(t) = v.get("type").and_then(|x| x.as_str()) else {
            continue;
        };

        if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()) {
            last_activity = ts.to_string();
        }
        if cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                cwd = c.to_string();
                project = c
                    .trim_end_matches(['/', '\\'])
                    .rsplit(['/', '\\'])
                    .next()
                    .unwrap_or(c)
                    .to_string();
            }
        }

        match t {
            "custom-title" => {
                if let Some(ct) = v.get("customTitle").and_then(|x| x.as_str()) {
                    custom_title = ct.to_string();
                }
            }
            // Claude Code 落盘的 AI 总结标题（若版本支持）
            "summary" => {
                if let Some(s) = v.get("summary").and_then(|x| x.as_str()) {
                    ai_title = s.to_string();
                }
            }
            "user" => parse_user_line(&v, &last_activity, &mut title, &mut preview, &mut messages),
            "assistant" => parse_assistant_line(&v, &last_activity, &mut preview, &mut messages),
            "system" => {
                if let Some(c) = v.get("content") {
                    if let Some(text) = content_to_text(c) {
                        messages.push(raw_message("system", "system", &text, &last_activity, None));
                    }
                }
            }
            _ => {}
        }
    }

    // 标题优先级：用户自定义 > AI 总结 > 首条用户输入
    let final_title = if !custom_title.is_empty() {
        custom_title
    } else if !ai_title.is_empty() {
        ai_title
    } else if !title.is_empty() {
        title
    } else {
        "(未命名会话)".to_string()
    };

    Some(SessionInfo {
        id: session_id,
        project: if project.is_empty() {
            "unknown".to_string()
        } else {
            project
        },
        cwd,
        title: final_title,
        status: "completed".to_string(),
        last_activity,
        preview,
        unread: false,
        messages,
    })
}

/// 组装一条历史消息。
fn raw_message(
    msg_type: &'static str,
    role: &'static str,
    content: &str,
    time: &str,
    tool_call: Option<String>,
) -> Message {
    Message {
        msg_type,
        role,
        content: content.to_string(),
        time: short_time(time),
        tool_call,
    }
}

/// 解析原生 transcript 中的 `user` 行：
/// - 字符串 content 为真实用户输入；系统注入的命令回显归为 system；
/// - 块数组 content 中的 `tool_result` 为工具执行结果、`text` 为用户文本。
fn parse_user_line(
    v: &serde_json::Value,
    time: &str,
    title: &mut String,
    preview: &mut String,
    messages: &mut Vec<Message>,
) {
    let Some(msg) = v.get("message") else { return };
    let Some(content) = msg.get("content") else { return };

    match content {
        serde_json::Value::String(s) => {
            let is_marker = is_system_marker(s);
            if title.is_empty() && !is_marker {
                *title = truncate(s, 40);
            }
            *preview = truncate(s, 80);
            messages.push(raw_message(
                if is_marker { "system" } else { "user" },
                if is_marker { "system" } else { "user" },
                s,
                time,
                None,
            ));
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(|x| x.as_str()) {
                    Some("tool_result") => {
                        if let Some(c) = block.get("content") {
                            if let Some(text) = content_to_text(c) {
                                *preview = truncate(&text, 80);
                                messages.push(raw_message("tool_result", "assistant", &text, time, None));
                            }
                        }
                    }
                    Some("text") => {
                        if let Some(text) = block_text(block) {
                            if title.is_empty() {
                                *title = truncate(&text, 40);
                            }
                            *preview = truncate(&text, 80);
                            messages.push(raw_message("user", "user", &text, time, None));
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// 解析原生 transcript 中的 `assistant` 行，忠实保留每个内容块：
/// - `text` 块 → assistant 文本；
/// - `thinking` 块 → 思考过程（thinking 类型）；
/// - `tool_use` 块 → 工具调用（带 toolCall 徽标与入参）；
/// - `tool_result` 块 → 工具结果。
fn parse_assistant_line(
    v: &serde_json::Value,
    time: &str,
    preview: &mut String,
    messages: &mut Vec<Message>,
) {
    let Some(msg) = v.get("message") else { return };
    let Some(content) = msg.get("content") else { return };

    match content {
        serde_json::Value::String(s) => {
            *preview = truncate(s, 80);
            messages.push(raw_message("assistant", "assistant", s, time, None));
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(|x| x.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                            *preview = truncate(t, 80);
                            messages.push(raw_message("assistant", "assistant", t, time, None));
                        }
                    }
                    Some("thinking") => {
                        if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                            messages.push(raw_message("thinking", "assistant", t, time, None));
                        }
                    }
                    Some("tool_use") => {
                        let name = block
                            .get("name")
                            .and_then(|x| x.as_str())
                            .unwrap_or("工具")
                            .to_string();
                        let input = block
                            .get("input")
                            .map(|i| i.to_string())
                            .filter(|s| s != "null" && !s.is_empty())
                            .unwrap_or_default();
                        let content = if input.is_empty() {
                            format!("调用工具 {name}")
                        } else {
                            format!("调用工具 {name}\n{input}")
                        };
                        messages.push(raw_message("tool_use", "assistant", &content, time, Some(name)));
                    }
                    Some("tool_result") => {
                        if let Some(c) = block.get("content") {
                            if let Some(text) = content_to_text(c) {
                                messages.push(raw_message("tool_result", "assistant", &text, time, None));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// 从文本类 block 中提取字符串（兼容 `text` 与 `content` 两种字段）。
fn block_text(block: &serde_json::Value) -> Option<String> {
    if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
        return Some(t.to_string());
    }
    block
        .get("content")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// 从 content 值提取纯文本：字符串 / 文本块数组 / 递归 `tool_result.content`。
fn content_to_text(content: &serde_json::Value) -> Option<String> {
    match content {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(arr) => {
            let mut parts: Vec<String> = Vec::new();
            for item in arr {
                match item {
                    serde_json::Value::String(s) => parts.push(s.clone()),
                    serde_json::Value::Object(o) => match o.get("type").and_then(|x| x.as_str()) {
                        Some("text") => {
                            if let Some(t) = o.get("text").and_then(|x| x.as_str()) {
                                parts.push(t.to_string());
                            }
                        }
                        Some("tool_result") => {
                            if let Some(c) = o.get("content") {
                                if let Some(t) = content_to_text(c) {
                                    parts.push(t);
                                }
                            }
                        }
                        _ => {}
                    },
                    _ => {}
                }
            }
            if parts.is_empty() {
                None
            } else {
                Some(parts.join("\n"))
            }
        }
        serde_json::Value::Object(o) => o.get("text").and_then(|x| x.as_str()).map(|s| s.to_string()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn append(dir: &Path, session: &str, line: &str) {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join(format!("event-{session}.jsonl"));
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .unwrap()
            .write_all(format!("{line}\n").as_bytes())
            .unwrap();
    }

    #[test]
    fn parses_sessions_and_statuses() {
        let dir = std::env::temp_dir().join("ccbuddy-test-events");
        let _ = std::fs::remove_dir_all(&dir);

        // 会话 A：等待确认
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-a","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-a","cwd":"D:/work/proj","prompt":"帮我修复支付回调"}}"#);
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PreToolUse","payload":{"session_id":"sess-a","cwd":"D:/work/proj","tool_name":"Bash"}}"#);

        // 会话 B：已完成
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-b","cwd":"D:/work/other"}}"#);
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-b","cwd":"D:/work/other","prompt":"生成文档"}}"#);
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"SessionEnd","payload":{"session_id":"sess-b","cwd":"D:/work/other"}}"#);

        let sessions = load_sessions_from(&dir);

        assert_eq!(sessions.len(), 2);
        let a = sessions.iter().find(|s| s.id == "sess-a").unwrap();
        let b = sessions.iter().find(|s| s.id == "sess-b").unwrap();

        assert_eq!(a.status, "waiting_confirmation");
        assert_eq!(a.title, "帮我修复支付回调");
        assert_eq!(a.project, "proj");
        assert!(a.unread);

        assert_eq!(b.status, "completed");
        assert_eq!(b.title, "生成文档");
        assert_eq!(b.project, "other");
        assert!(!b.unread);

        // 消息应包含用户提示词和工具调用
        assert!(a.messages.iter().any(|m| m.msg_type == "user"));
        assert!(a.messages.iter().any(|m| m.tool_call.as_deref() == Some("Bash")));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
