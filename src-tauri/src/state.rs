use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use serde::Serialize;

use crate::event::Event;

/// 事件流每个会话保留的最新事件条数。
const MAX_EVENTS_PER_SESSION: usize = 50;

/// 单个会话的解析缓存：文件未变化（mtime 相同）时直接复用上次结果，
/// 轮询刷新只重读有更新的日志文件，避免每次全量解析。
struct CachedSession {
    mtime: SystemTime,
    info: SessionInfo,
}

fn event_cache() -> &'static Mutex<HashMap<String, CachedSession>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedSession>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

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
    /// 消息列表。列表查询（懒加载）时为空，由 `get_session_detail` 按需填充。
    #[serde(default)]
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
        self.last_activity = to_local_iso(&ev.received_at);

        if let Some(cwd) = &ev.cwd {
            self.cwd = cwd.clone();
            self.set_project_from_cwd(cwd);
        }

        match ev.hook_event.as_str() {
            "SessionStart" | "Setup" => {
                self.status = SessionStatus::Idle;
            }
            "UserPromptSubmit" => {
                // 新一轮 prompt 开始：重置上一轮的错误状态，避免 error 粘滞到会话结束
                self.has_error = false;
                self.status = SessionStatus::Running;
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
            }
            "UserPromptExpansion" => {
                // 用户提示词被 Claude Code 扩展：展示扩展结果（不影响标题）
                let text = ev
                    .payload
                    .get("expanded_prompt")
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(|s| s.to_string())
                    .or_else(|| ev.prompt());
                if let Some(t) = text {
                    if !is_system_marker(&t) {
                        self.preview = truncate(&t, 60);
                        self.messages.push(Message {
                            msg_type: "user",
                            role: "user",
                            content: t,
                            time: short_time(&ev.received_at),
                            tool_call: None,
                        });
                    }
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
            "PostToolUseFailure" => {
                self.status = SessionStatus::Error;
                self.has_error = true;
                self.messages.push(system_message("工具调用失败", &ev.received_at));
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
            "StopFailure" => {
                self.status = SessionStatus::Error;
                self.has_error = true;
                self.messages.push(system_message("会话停止失败", &ev.received_at));
            }
            "SessionEnd" => {
                if !self.has_error {
                    self.status = SessionStatus::Completed;
                }
            }
            // 可观测元事件（权限/压缩/任务/文件变更等）：作为 system 消息展示，不改变核心状态。
            // 其余事件（MessageDisplay 等）：若带文本内容则作为 assistant 消息展示。
            _ => {
                if let Some(desc) = meta_description(ev) {
                    self.messages.push(system_message(&desc, &ev.received_at));
                } else if let Some(content) = ev.message() {
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

/// 为可观测元事件生成一句话描述（权限 / 压缩 / 任务 / 文件变更 / 子代理等）。
///
/// 这些事件不改变会话的核心状态机，只在事件流时间线里作为 system 消息展示。
/// 提取不到特定字段时回退到事件名；未知事件返回 None（由调用方走通用消息分支）。
fn meta_description(ev: &Event) -> Option<String> {
    let s = |k: &str| {
        ev.payload
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
    };
    Some(match ev.hook_event.as_str() {
        "ConfigChange" => s("config_source")
            .map(|v| format!("配置变更：{v}"))
            .unwrap_or_else(|| "配置变更".to_string()),
        "CwdChanged" => s("cwd")
            .map(|v| format!("工作目录切换：{v}"))
            .unwrap_or_else(|| "工作目录切换".to_string()),
        "DirectoryAdded" => s("directory")
            .map(|v| format!("新增目录：{v}"))
            .unwrap_or_else(|| "新增目录".to_string()),
        "FileChanged" => s("file_path")
            .map(|v| format!("文件变更：{v}"))
            .unwrap_or_else(|| "文件变更".to_string()),
        "InstructionsLoaded" => s("file_path")
            .map(|v| format!("加载指令：{v}"))
            .unwrap_or_else(|| "加载指令".to_string()),
        "WorktreeCreate" => s("name")
            .map(|v| format!("创建工作树：{v}"))
            .unwrap_or_else(|| "创建工作树".to_string()),
        "WorktreeRemove" => s("worktree_path")
            .or_else(|| s("path"))
            .map(|v| format!("移除工作树：{v}"))
            .unwrap_or_else(|| "移除工作树".to_string()),
        "PreCompact" => "开始压缩上下文".to_string(),
        "PostCompact" => "上下文压缩完成".to_string(),
        "TaskCreated" => s("task_name")
            .or_else(|| s("task_id"))
            .map(|v| format!("创建任务：{v}"))
            .unwrap_or_else(|| "创建任务".to_string()),
        "TaskCompleted" => s("task_name")
            .or_else(|| s("task_id"))
            .map(|v| format!("任务完成：{v}"))
            .unwrap_or_else(|| "任务完成".to_string()),
        "TeammateIdle" => "队友空闲".to_string(),
        "SubagentStart" => s("subagent_name")
            .map(|v| format!("启动子代理：{v}"))
            .unwrap_or_else(|| "启动子代理".to_string()),
        "PermissionRequest" => {
            let tool = s("tool_name").unwrap_or_else(|| "工具".to_string());
            match s("reason") {
                Some(r) => format!("请求权限：{tool}（{r}）"),
                None => format!("请求权限：{tool}"),
            }
        }
        "PermissionDenied" => s("tool_name")
            .map(|v| format!("权限被拒：{v}"))
            .unwrap_or_else(|| "权限被拒".to_string()),
        "Elicitation" => ev
            .message()
            .map(|m| format!("MCP 请求输入：{}", truncate(&m, 60)))
            .unwrap_or_else(|| "MCP 请求输入".to_string()),
        "ElicitationResult" => s("response")
            .map(|v| format!("MCP 输入结果：{}", truncate(&v, 60)))
            .unwrap_or_else(|| "MCP 输入结果".to_string()),
        "PostToolBatch" => "工具批次执行完成".to_string(),
        _ => return None,
    })
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

/// 把 ISO 时间戳（UTC "Z" 或带偏移的本地时间）归一为本地时区时间。
///
/// hook 日志新格式为本地时间（带偏移），旧格式为 UTC；原生 transcript 为 UTC。
/// 统一转为本地时间后，字符串排序与 HH:MM 截取（`short_time`）才正确。
/// 解析失败时原样返回（容忍脏数据）。
fn to_local_iso(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        Err(_) => iso.to_string(),
    }
}

/// 从 ISO 时间戳提取 HH:MM 短格式。
fn short_time(iso: &str) -> String {
    // received_at 形如 "2026-08-20T14:35:22.123Z"，取 "T" 后的前 5 个字符。
    iso.split('T')
        .nth(1)
        .map(|t| t.chars().take(5).collect())
        .unwrap_or_else(|| iso.to_string())
}

/// 日志源目录：`~/.ccbuddy/events`（hook 事件流，不提供配置项）。
pub fn events_dir() -> PathBuf {
    crate::config::data_root().join("events")
}

/// Claude 配置目录（用户可在设置页配置，默认 `~/.claude`）。
pub fn claude_dir() -> PathBuf {
    crate::config::claude_dir()
}

/// 检测 hook 安装/注册状态（设置页展示）。
///
/// 返回 JSON：`{ installed: bool, registered: { <事件名>: bool, ... } }`
/// - installed：`~/.claude/ccbuddy-hook[.exe]` 可执行文件存在
/// - registered：`settings.json` 的 hooks.<事件> 下存在指向该 hook 的 command
pub fn hook_status() -> serde_json::Value {
    let hook_name = crate::hook_file_name();
    let claude = claude_dir();
    let installed = claude.join(hook_name).is_file();

    let mut registered = serde_json::Map::new();
    let settings = claude.join("settings.json");
    let root: serde_json::Value = std::fs::read_to_string(&settings)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null);

    let command = claude
        .join(hook_name)
        .to_string_lossy()
        .replace('\\', "/");
    if let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) {
        for ev in crate::HOOK_EVENTS {
            let hit = hooks
                .get(ev)
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .any(|e| crate::entry_has_command(e, &command))
                })
                .unwrap_or(false);
            registered.insert(ev.to_string(), serde_json::Value::Bool(hit));
        }
    }

    serde_json::json!({
        "installed": installed,
        "registered": registered,
        // hook 手动下载地址（离线环境用，latest release 固定链接）
        "downloadUrl": crate::hook_download_url(),
    })
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

/// 事件流会话列表（hook 日志 `~/.ccbuddy/events`，实时会话，状态由状态机推断）。
///
/// 增量刷新：每个会话文件按 mtime 缓存解析结果，只有更新的文件才重新读取；
/// 每个会话只保留最新 [`MAX_EVENTS_PER_SESSION`] 条事件。
pub fn load_sessions() -> Vec<SessionInfo> {
    let dir = events_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut cache = event_cache().lock().unwrap();
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for entry in rd.flatten() {
        let path = entry.path();
        let session_id = session_id_from_filename(&path);
        if session_id.is_empty() {
            continue;
        }
        seen.insert(session_id.clone());
        let Some(mtime) = file_mtime(&path) else {
            continue;
        };

        let info = match cache.get(&session_id) {
            // 文件未变化：直接复用上次解析结果
            Some(c) if c.mtime == mtime => c.info.clone(),
            _ => {
                let mut info = parse_event_file(&path, &session_id);
                // 只读尾部 N 条时，首条用户输入（标题来源）可能不在窗口内，沿用旧标题
                if let Some(prev) = cache.get(&session_id) {
                    if info.title == "(未命名会话)" && prev.info.title != "(未命名会话)" {
                        info.title = prev.info.title.clone();
                    }
                    if info.cwd.is_empty() {
                        info.cwd = prev.info.cwd.clone();
                        info.project = prev.info.project.clone();
                    }
                }
                cache.insert(session_id.clone(), CachedSession { mtime, info: info.clone() });
                info
            }
        };
        out.push(info);
    }

    // 清理日志文件已删除的会话缓存
    cache.retain(|k, _| seen.contains(k));

    out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    out
}

/// 历史会话列表（Claude Code 原生 transcript，`<claude_dir>/projects/`）。
///
/// 与事件流分开：历史界面只取原生数据，不混合 hook 日志。
pub fn load_history_sessions() -> Vec<SessionInfo> {
    let mut out = load_native_sessions();
    out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    out
}

/// 紧急会话（等待确认 / 等待输入 / 出错）的 id 列表（任务栏通知用）。
#[cfg(feature = "gui")]
pub fn urgent_session_ids() -> Vec<String> {
    load_sessions()
        .into_iter()
        .filter(|s| {
            matches!(
                s.status.as_str(),
                "waiting_confirmation" | "waiting_input" | "error"
            )
        })
        .map(|s| s.id)
        .collect()
}

/// 按需加载事件流会话详情（hook 日志 `~/.ccbuddy/events`，最新 50 条事件）。
pub fn load_event_detail(session_id: &str) -> Option<SessionInfo> {
    let path = events_dir().join(format!("event-{session_id}.jsonl"));
    if path.is_file() {
        Some(parse_event_file(&path, session_id))
    } else {
        None
    }
}

/// 按需加载历史会话详情（原生 transcript，全量消息）。
pub fn load_session_detail(session_id: &str) -> Option<SessionInfo> {
    let path = native_session_path(session_id)?;
    parse_native_session(&path, false)
}

/// 文件修改时间。
fn file_mtime(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

/// 解析单个事件日志文件 `event-<session_id>.jsonl`，聚合为会话信息。
///
/// 只读文件尾部（最新 [`MAX_EVENTS_PER_SESSION`] 条事件），大文件不必全量读入。
fn parse_event_file(path: &Path, session_id: &str) -> SessionInfo {
    let lines = read_tail_lines(path, MAX_EVENTS_PER_SESSION);
    let mut agg: Option<SessionAgg> = None;
    for line in &lines {
        if let Some(ev) = Event::parse(line, session_id) {
            // 会话 id 优先取 payload 中的值，缺失时用文件名提取的 id
            let a = agg.get_or_insert_with(|| SessionAgg::new(ev.session_id.clone()));
            a.apply(&ev);
        }
    }
    agg.map(SessionAgg::into_info)
        .unwrap_or_else(|| SessionAgg::new(session_id.to_string()).into_info())
}

/// 读取文件尾部的完整行（最多 `max_lines` 条）。
///
/// 大文件只读最后 512KB；尾部块内行数不足时（单行超长）回退全量读取。
fn read_tail_lines(path: &Path, max_lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    const CHUNK: u64 = 512 * 1024;
    let collect = |text: &str| -> Vec<String> {
        text.lines()
            .map(str::trim)
            .filter(|l| !l.is_empty())
            .map(String::from)
            .collect()
    };

    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = f.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    if len <= CHUNK {
        let mut buf = String::new();
        f.read_to_string(&mut buf).ok();
        return collect(&buf);
    }

    f.seek(SeekFrom::Start(len - CHUNK)).ok();
    let mut buf = Vec::new();
    f.take(CHUNK).read_to_end(&mut buf).ok();
    // 首行可能从多字节字符中间被截断，用 lossy 转换并丢弃首行
    let text = String::from_utf8_lossy(&buf);
    let mut lines = collect(&text);
    if !lines.is_empty() {
        lines.remove(0);
    }
    if lines.len() < max_lines {
        let mut buf = String::new();
        if std::fs::File::open(path)
            .and_then(|mut f2| f2.read_to_string(&mut buf))
            .is_ok()
        {
            lines = collect(&buf);
        }
    }
    lines
}

/// 在 `~/.claude/projects/` 下查找会话对应的 transcript 文件路径。
fn native_session_path(session_id: &str) -> Option<PathBuf> {
    let dir = projects_dir();
    let Ok(project_dirs) = std::fs::read_dir(&dir) else {
        return None;
    };
    for project_dir in project_dirs.flatten() {
        let path = project_dir.path().join(format!("{session_id}.jsonl"));
        if path.is_file() {
            return Some(path);
        }
    }
    None
}

/// Claude Code 原生会话目录：`~/.claude/projects/<项目路径编码>/<session-id>.jsonl`。
fn projects_dir() -> PathBuf {
    claude_dir().join("projects")
}

/// 扫描原生历史会话，聚合为会话列表（历史会话统一标记为 completed）。
///
/// 概要模式（lazy=true）：不收集消息，只为列表提供标题/项目/时间/预览。
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
            // lazy=true：列表概要模式，不收集消息（详情由 get_session_detail 按需解析）
            if let Some(info) = parse_native_session(&fp, true) {
                result.push(info);
            }
        }
    }
    result
}

/// 解析单个原生会话文件（Claude Code 的 transcript .jsonl）。
///
/// `lazy = true`：概要模式（列表用），只提取标题/项目/时间/预览，不收集消息；
/// `false`：解析全部消息（用户点开会话详情时）。
fn parse_native_session(path: &Path, lazy: bool) -> Option<SessionInfo> {
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
            last_activity = to_local_iso(ts);
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
            "user" => parse_user_line(&v, &last_activity, &mut title, &mut preview, &mut messages, lazy),
            "assistant" => parse_assistant_line(&v, &last_activity, &mut preview, &mut messages, lazy),
            "system" => {
                if let Some(c) = v.get("content") {
                    if let Some(text) = content_to_text(c) {
                        if !lazy {
                            messages.push(raw_message("system", "system", &text, &last_activity, None));
                        }
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
    lazy: bool,
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
            if !lazy {
                messages.push(raw_message(
                    if is_marker { "system" } else { "user" },
                    if is_marker { "system" } else { "user" },
                    s,
                    time,
                    None,
                ));
            }
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(|x| x.as_str()) {
                    Some("tool_result") => {
                        if let Some(c) = block.get("content") {
                            if let Some(text) = content_to_text(c) {
                                *preview = truncate(&text, 80);
                                if !lazy {
                                    messages.push(raw_message("tool_result", "assistant", &text, time, None));
                                }
                            }
                        }
                    }
                    Some("text") => {
                        if let Some(text) = block_text(block) {
                            if title.is_empty() {
                                *title = truncate(&text, 40);
                            }
                            *preview = truncate(&text, 80);
                            if !lazy {
                                messages.push(raw_message("user", "user", &text, time, None));
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
    lazy: bool,
) {
    let Some(msg) = v.get("message") else { return };
    let Some(content) = msg.get("content") else { return };

    match content {
        serde_json::Value::String(s) => {
            *preview = truncate(s, 80);
            if !lazy {
                messages.push(raw_message("assistant", "assistant", s, time, None));
            }
        }
        serde_json::Value::Array(blocks) => {
            for block in blocks {
                match block.get("type").and_then(|x| x.as_str()) {
                    Some("text") => {
                        if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                            *preview = truncate(t, 80);
                            if !lazy {
                                messages.push(raw_message("assistant", "assistant", t, time, None));
                            }
                        }
                    }
                    Some("thinking") => {
                        if !lazy {
                            if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                                messages.push(raw_message("thinking", "assistant", t, time, None));
                            }
                        }
                    }
                    Some("tool_use") => {
                        if !lazy {
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
                    }
                    Some("tool_result") => {
                        if !lazy {
                            if let Some(c) = block.get("content") {
                                if let Some(text) = content_to_text(c) {
                                    messages.push(raw_message("tool_result", "assistant", &text, time, None));
                                }
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

        let sessions: Vec<SessionInfo> = ["sess-a", "sess-b"]
            .iter()
            .map(|id| parse_event_file(&dir.join(format!("event-{id}.jsonl")), id))
            .collect();

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

    #[test]
    fn parses_failure_and_meta_events() {
        let dir = std::env::temp_dir().join("ccbuddy-test-new-events");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-fail","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-fail","prompt":"编译项目"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PostToolUseFailure","payload":{"session_id":"sess-fail","tool_name":"Bash"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"FileChanged","payload":{"session_id":"sess-fail","file_path":"D:/work/proj/.envrc"}}"#);

        let info = parse_event_file(&dir.join("event-sess-fail.jsonl"), "sess-fail");
        assert_eq!(info.status, "error");
        assert!(info.unread);
        assert!(info.messages.iter().any(|m| m.msg_type == "system" && m.content.contains("工具调用失败")));
        assert!(info.messages.iter().any(|m| m.content.contains("文件变更") && m.content.contains(".envrc")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn meta_description_extracts_fields() {
        let ev = Event::parse(
            r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"FileChanged","payload":{"session_id":"s","file_path":"/a/b.env"}}"#,
            "fallback",
        )
        .unwrap();
        assert_eq!(meta_description(&ev).unwrap(), "文件变更：/a/b.env");

        let ev2 = Event::parse(
            r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"PostToolBatch","payload":{"session_id":"s"}}"#,
            "fallback",
        )
        .unwrap();
        assert_eq!(meta_description(&ev2).unwrap(), "工具批次执行完成");

        let ev3 = Event::parse(
            r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"AssistantMessage","payload":{"session_id":"s","message":"hi"}}"#,
            "fallback",
        )
        .unwrap();
        assert_eq!(meta_description(&ev3), None);
    }
}
