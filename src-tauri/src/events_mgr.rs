//! 事件流管理：解析 hook 日志（`~/.ccbuddy/events/event-<session-id>.jsonl`）。
//!
//! 数据来源是 `ccbuddy-hook` 写入的事件流日志：
//! - `load_events`：事件流会话列表（增量刷新，按 mtime 缓存，每会话最新 50 条事件）
//! - `load_event_detail`：单个会话详情（按需解析）
//! - 事件序列（SessionStart / UserPromptSubmit / PreToolUse / …）驱动状态机，
//!   推断每个会话的最新状态（运行中 / 等待确认 / 等待输入 / 出错 / 已完成）
//! - `hook_status`：hook 安装/注册状态检测（设置页展示）
//!
//! 与原生 transcript 解析（[`crate::claude`]）分开：事件流是 hook 采集的实时数据，
//! 历史会话是 Claude Code 自己落盘的记录。共享的展示模型与工具函数见 [`crate::utils`]。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::config::{claude_dir, events_dir};
use crate::utils::{
    is_system_marker, project_name_from_cwd, read_settings_json, short_time, to_local_iso,
    truncate, Message, SessionInfo, UNNAMED_SESSION, UNKNOWN_PROJECT,
};
use crate::event::Event;

/// 事件流每个会话保留的最新事件条数。
const MAX_EVENTS_PER_SESSION: usize = 50;

/// 活跃状态时效下限（秒）：`running` / `waiting_confirmation` / `waiting_input`
/// 超过该时长无新事件（按 `last_activity` 与当前时间差）即降级为 `idle`。
///
/// Esc 打断 / 关终端 / 崩溃时没有终止事件，状态会永久卡在"活跃"，
/// 这里做纯本地推断的兜底。10 分钟为经验值，可按需调整。
const STALE_ACTIVE_AFTER_SECS: i64 = 600;

/// 单个会话的解析缓存：文件未变化（mtime + size 都相同）时直接复用上次
/// 结果，轮询刷新只重读有更新的日志文件，避免每次全量解析。
///
/// M-B11：mtime 之外同时比对文件 size——NTFS / drvfs 的时间戳粒度是
/// 1-2 秒，同秒内多次追加 mtime 不变，但 append 只增不减，size 必能探测。
struct CachedSession {
    mtime: SystemTime,
    size: u64,
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

/// 前端展示用的单条消息与 SessionInfo 定义在 [`crate::utils`]（共享模型）。

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
    /// 原生 transcript 路径（事件里首次出现时记录，L-B12）。
    transcript_path: Option<String>,
    /// SessionStart 来源：startup / resume / clear / compact / fork（L-B12）。
    source: Option<String>,
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
            transcript_path: None,
            source: None,
        }
    }

    fn set_project_from_cwd(&mut self, cwd: &str) {
        if !cwd.is_empty() {
            // 与原生 transcript 解析共用同一条取项目名规则（见 utils）
            self.project = project_name_from_cwd(cwd);
        }
    }

    /// 根据一个事件推进状态机。
    ///
    /// 本体只做三件事：更新「每个事件都会动」的公共字段（时间 / 工作目录 /
    /// 子代理隔离 / transcript 路径），然后按事件名把控制权交给对应的
    /// `on_*` 转移（每个转移是独立的、可单读的小函数）。
    fn apply(&mut self, ev: &Event) {
        if self.observe_common(ev) {
            return; // 子代理事件：已就地处理，不参与主会话状态机
        }

        match ev.hook_event.as_str() {
            "SessionStart" | "Setup" => self.on_session_start(ev),
            "UserPromptSubmit" => self.on_user_prompt(ev),
            "UserPromptExpansion" => self.on_prompt_expansion(ev),
            "PreToolUse" => self.on_pre_tool_use(ev),
            "PostToolUse" => self.on_post_tool_use(),
            "PostToolUseFailure" => self.on_post_tool_use_failure(ev),
            "Notification" => self.on_notification(ev),
            "PermissionRequest" => self.on_permission_request(ev),
            "PermissionDenied" => self.on_permission_denied(ev),
            "Stop" => self.on_stop(),
            // 子代理停止：带 parent_tool_use_id 的已在 observe_common 里隔离（M-B9）；
            // 主会话自身的 SubagentStop 不改变状态
            "SubagentStop" => {}
            "StopFailure" => self.on_stop_failure(ev),
            "SessionEnd" => self.on_session_end(ev),
            // 其余事件：可观测元事件（权限/压缩/任务/文件变更等）与 MessageDisplay
            _ => self.on_other(ev),
        }
    }

    /// 公共前置：每个事件都要过一遍的字段维护。
    ///
    /// 返回前若判定为子代理事件（M-B9：`parent_tool_use_id` 非空）则已就地
    /// 处理完毕并标记 `handled`，调用方应停止后续状态转移。
    fn observe_common(&mut self, ev: &Event) -> bool {
        self.last_activity = to_local_iso(&ev.received_at);

        if let Some(cwd) = &ev.cwd {
            self.cwd = cwd.clone();
            self.set_project_from_cwd(cwd);
        }

        // M-B9：子代理事件与主会话共用 session_id，但不属于主会话状态机：
        // 仅作为时间线上一条简短 system 消息展示，不驱动状态转移、
        // 不更新 preview / title。
        if ev.parent_tool_use_id.is_some() {
            let tool = ev.tool_name().unwrap_or_else(|| ev.hook_event.clone());
            self.messages.push(system_message(
                &format!("子代理活动：{tool}"),
                &ev.received_at,
            ));
            return true;
        }

        // L-B12：首次见到 transcript_path 时记录（后续事件重复携带不覆盖）
        if self.transcript_path.is_none() {
            self.transcript_path = ev.transcript_path.clone();
        }
        false
    }

    /// 追加一条展示消息。
    fn push_message(
        &mut self,
        msg_type: &'static str,
        role: &'static str,
        content: &str,
        time: &str,
        tool_call: Option<String>,
    ) {
        self.messages.push(Message {
            msg_type,
            role,
            content: content.to_string(),
            time: short_time(time),
            tool_call,
        });
    }

    /// SessionStart / Setup：新会话开始，置空闲并记住来源。
    fn on_session_start(&mut self, ev: &Event) {
        self.status = SessionStatus::Idle;
        if let Some(src) = &ev.source {
            self.source = Some(src.clone());
        }
    }

    /// UserPromptSubmit：新一轮提示词开始。
    fn on_user_prompt(&mut self, ev: &Event) {
        // 重置上一轮的错误状态，避免 error 粘滞到会话结束
        self.has_error = false;
        self.status = SessionStatus::Running;
        let Some(prompt) = ev.prompt() else { return };
        if is_system_marker(&prompt) {
            return;
        }
        if self.title.is_empty() {
            self.title = truncate(&prompt, 30);
        }
        self.preview = truncate(&prompt, 60);
        self.push_message("user", "user", &prompt, &ev.received_at, None);
    }

    /// UserPromptExpansion：展示扩展后的提示词（不影响标题）。
    ///
    /// 优先取 `expanded_prompt` 字段；没有（或为空）时回退到原始 prompt。
    fn on_prompt_expansion(&mut self, ev: &Event) {
        let expanded: Option<&str> = ev
            .payload
            .get("expanded_prompt")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty());
        let text: Option<String> = match expanded {
            Some(s) => Some(s.to_string()),
            None => ev.prompt(),
        };
        let Some(t) = text else { return };
        if is_system_marker(&t) {
            return;
        }
        self.preview = truncate(&t, 60);
        self.push_message("user", "user", &t, &ev.received_at, None);
    }

    /// PreToolUse：工具调用开始。只更新展示并回到 Running。
    ///
    /// 官方语义：PreToolUse 在每次工具调用前都触发（无论是否弹权限窗），
    /// 不能据此判定「等待确认」——「等待确认」由 PermissionRequest 驱动。
    fn on_pre_tool_use(&mut self, ev: &Event) {
        let tool = ev.tool_name().unwrap_or_else(|| "工具".to_string());
        // 预览保持短句（左侧列表一行显示）；消息内容补上入参概要，供事件流工具行展示
        // （首行"调用工具 X"，其后一行概要），避免工具行只剩名称显得空。
        let brief = format!("调用工具 {tool}…");
        self.preview = brief.clone();
        // 入参可能是整份文件内容（Write / Edit），只留限长概要，不落原始 JSON
        let content = match ev.payload.get("tool_input") {
            Some(v) if v.is_object() => format!("调用工具 {tool}\n{}", tool_brief(v)),
            _ => format!("调用工具 {tool}"),
        };
        self.push_message("assistant", "assistant", &content, &ev.received_at, Some(tool));
        self.status = SessionStatus::Running;
    }

    /// PostToolUse：工具执行完成，清除「等待确认」（权限被批准路径）。
    ///
    /// 出错状态不覆盖（失败检测由 PostToolUseFailure 负责）。
    fn on_post_tool_use(&mut self) {
        if self.status != SessionStatus::Error {
            self.status = SessionStatus::Running;
        }
    }

    /// PostToolUseFailure：工具调用失败。
    fn on_post_tool_use_failure(&mut self, ev: &Event) {
        self.status = SessionStatus::Error;
        self.has_error = true;
        self.messages.push(system_message("工具调用失败", &ev.received_at));
    }

    /// Notification：只在文案明确指向「等用户输入」时置等待输入。
    fn on_notification(&mut self, ev: &Event) {
        let Some(msg) = ev.message() else { return };
        if is_input_request(&msg) {
            self.status = SessionStatus::WaitingInput;
            self.preview = truncate(&msg, 60);
        }
    }

    /// PermissionRequest：权限弹窗弹出，驱动「等待确认」。
    fn on_permission_request(&mut self, ev: &Event) {
        self.status = SessionStatus::WaitingConfirmation;
        let desc = meta_description(ev).unwrap_or_else(|| "请求权限".to_string());
        self.messages.push(system_message(&desc, &ev.received_at));
    }

    /// PermissionDenied：权限被拒，清除「等待确认」。
    ///
    /// 不标记错误——用户主动拒绝不是故障。
    fn on_permission_denied(&mut self, ev: &Event) {
        if self.status == SessionStatus::WaitingConfirmation {
            self.status = SessionStatus::Running;
        }
        let desc = meta_description(ev).unwrap_or_else(|| "权限被拒".to_string());
        self.messages.push(system_message(&desc, &ev.received_at));
    }

    /// Stop：本轮响应结束（非会话结束）。
    ///
    /// 非错误状态下标记 Completed；出错时保持 Error 不动。
    fn on_stop(&mut self) {
        if !self.has_error {
            self.status = SessionStatus::Completed;
        }
    }

    /// StopFailure：会话异常停止。
    fn on_stop_failure(&mut self, ev: &Event) {
        self.status = SessionStatus::Error;
        self.has_error = true;
        self.messages.push(system_message("会话停止失败", &ev.received_at));
    }

    /// SessionEnd：按 reason 区分（L-B13）。
    ///
    /// - `clear`：用户执行 /clear，上下文已清空，会话仍会继续
    ///   （随后有新 SessionStart）→ 置 Idle；
    /// - `logout` / `prompt_input_exit`：会话真正结束 → Completed；
    /// - 缺失 / other：维持原有 Completed 逻辑。
    fn on_session_end(&mut self, ev: &Event) {
        match ev.reason.as_deref() {
            Some("clear") => {
                self.status = SessionStatus::Idle;
                self.messages
                    .push(system_message("上下文已清空", &ev.received_at));
            }
            _ => {
                if !self.has_error {
                    self.status = SessionStatus::Completed;
                }
            }
        }
    }

    /// 其余事件：元事件（权限/压缩/任务/文件变更等）作为 system 消息展示，
    /// 不改变核心状态；带文本内容的（MessageDisplay 等）作为 assistant 消息展示。
    fn on_other(&mut self, ev: &Event) {
        if let Some(desc) = meta_description(ev) {
            self.messages.push(system_message(&desc, &ev.received_at));
            return;
        }
        let Some(content) = ev.message() else { return };
        self.preview = truncate(&content, 60);
        self.push_message("assistant", "assistant", &content, &ev.received_at, None);
        // 已处于「等用户动作」或出错状态时不改状态：这些状态比 Running 更需要保持
        if !matches!(
            self.status,
            SessionStatus::WaitingConfirmation | SessionStatus::WaitingInput | SessionStatus::Error
        ) {
            self.status = SessionStatus::Running;
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
            UNNAMED_SESSION.to_string()
        } else {
            self.title
        };
        let project = if self.project.is_empty() {
            UNKNOWN_PROJECT.to_string()
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
            transcript_path: self.transcript_path,
            source: self.source,
            subagents: Vec::new(),
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

/// 工具入参里最能表达意图的字段，按优先级取第一个非空字符串。
///
/// 与前端 `SessionDetail.vue` 的 `inputBrief` 键序一致（历史会话侧是同名逻辑）。
/// 必须显式取值而不能直接展示入参 JSON：serde_json 未开 `preserve_order`，
/// 键按字母序输出，`{"content":…}` 会排在 `file_path` 前面——Write 类调用会
/// 把整份文件内容顶到概要位置。
const TOOL_BRIEF_KEYS: [&str; 8] = [
    "file_path",
    "command",
    "url",
    "pattern",
    "path",
    "description",
    "query",
    "content",
];

/// 工具入参概要的字符上限。入参可能是整份文件内容（Write / Edit），必须限长：
/// 每个会话保留 [`MAX_EVENTS_PER_SESSION`] 条事件且每次推送全量下发，
/// 不限长时单条消息可达数百 KB。
const TOOL_BRIEF_MAX: usize = 120;

/// 取工具入参的一行概要：优先结构化字段，取不到则回退整个入参的紧凑 JSON。
fn tool_brief(input: &serde_json::Value) -> String {
    for key in TOOL_BRIEF_KEYS {
        if let Some(s) = input.get(key).and_then(|v| v.as_str()) {
            if !s.is_empty() {
                return truncate(s, TOOL_BRIEF_MAX);
            }
        }
    }
    truncate(&input.to_string(), TOOL_BRIEF_MAX)
}

/// 时效降级兜底（Esc 打断 / 关终端 / 崩溃时没有终止事件，活跃状态会永久卡死）。
///
/// `running` / `waiting_confirmation` / `waiting_input` 超过
/// [`STALE_ACTIVE_AFTER_SECS`] 无新事件（按 `last_activity` 与当前时间差）
/// 时降级为 `idle`。纯本地推断，不涉及任何远端状态。
/// `last_activity` 是 [`to_local_iso`] 的输出（本地时区 ISO 带偏移，可按
/// RFC3339 解析）；解析失败（脏数据 / 空值）时宁可不降级也不误降级。
fn downgrade_stale(info: &mut SessionInfo) {
    let active = matches!(
        info.status.as_str(),
        "running" | "waiting_confirmation" | "waiting_input"
    );
    if !active {
        return;
    }
    let Ok(last) = chrono::DateTime::parse_from_rfc3339(&info.last_activity) else {
        return; // 时间解析失败：不降级
    };
    let age_secs = chrono::Local::now()
        .signed_duration_since(last.with_timezone(&chrono::Local))
        .num_seconds();
    if age_secs > STALE_ACTIVE_AFTER_SECS {
        info.status = "idle".to_string();
        info.unread = false;
    }
}

/// 为可观测元事件生成一句话描述（权限 / 压缩 / 任务 / 文件变更 / 子代理等）。
///
/// 这些事件不改变会话的核心状态机，只在事件流时间线里作为 system 消息展示。
/// 提取不到特定字段时回退到事件名；未知事件返回 None（由调用方走通用消息分支）。
fn meta_description(ev: &Event) -> Option<String> {
    // 从 payload 取非空字符串字段（`Option<&str>` → `Option<String>`）
    let s = |k: &str| {
        ev.payload
            .get(k)
            .and_then(|v| v.as_str())
            .filter(|v| !v.is_empty())
            .map(|v| v.to_string())
    };
    // 拼成 "前缀：值"；值缺失时只有前缀
    let fmt = |prefix: &str, val: Option<String>| match val {
        Some(v) => format!("{prefix}：{v}"),
        None => prefix.to_string(),
    };

    Some(match ev.hook_event.as_str() {
        "ConfigChange" => fmt("配置变更", s("config_source")),
        "CwdChanged" => fmt("工作目录切换", s("cwd")),
        "DirectoryAdded" => fmt("新增目录", s("directory")),
        "FileChanged" => fmt("文件变更", s("file_path")),
        "InstructionsLoaded" => fmt("加载指令", s("file_path")),
        "WorktreeCreate" => fmt("创建工作树", s("name")),
        "WorktreeRemove" => fmt("移除工作树", s("worktree_path").or_else(|| s("path"))),
        "PreCompact" => "开始压缩上下文".to_string(),
        "PostCompact" => "上下文压缩完成".to_string(),
        "TaskCreated" => fmt("创建任务", s("task_name").or_else(|| s("task_id"))),
        "TaskCompleted" => fmt("任务完成", s("task_name").or_else(|| s("task_id"))),
        "TeammateIdle" => "队友空闲".to_string(),
        "SubagentStart" => fmt("启动子代理", s("subagent_name")),
        "PermissionRequest" => {
            let tool = s("tool_name").unwrap_or_else(|| "工具".to_string());
            match s("reason") {
                Some(r) => format!("请求权限：{tool}（{r}）"),
                None => format!("请求权限：{tool}"),
            }
        }
        "PermissionDenied" => fmt("权限被拒", s("tool_name")),
        "Elicitation" => match ev.message() {
            Some(m) => format!("MCP 请求输入：{}", truncate(&m, 60)),
            None => "MCP 请求输入".to_string(),
        },
        "ElicitationResult" => match s("response") {
            Some(v) => format!("MCP 输入结果：{}", truncate(&v, 60)),
            None => "MCP 输入结果".to_string(),
        },
        "PostToolBatch" => "工具批次执行完成".to_string(),
        // 未知事件：返回 None，调用方走通用消息分支
        _ => return None,
    })
}

/// 通知消息是否明确要求用户输入（M-B8 修正版启发式，即原 needs_input）。
///
/// 返回语义：true → 会话标记 WaitingInput。
///
/// 设计原则：Notification 仅作兜底信号（PermissionRequest 才是"等待确认"的
/// 权威来源），所以这里只匹配**明确的输入等待**模式，宁可漏判也不误判：
/// - 官方英文文案："waiting for your input" → 命中；
/// - "needs your permission" / 含 "permission" → 明确排除（那是等待确认，
///   由 PermissionRequest 事件负责，不能算等待输入）；
/// - 中文关键词保留（兼容用户中文环境 hook 与未来本地化），但收窄到
///   明确的输入等待类词汇，不再用"需要 / 请"这类到处都会出现的宽泛词。
fn is_input_request(msg: &str) -> bool {
    let lower = msg.to_lowercase();

    // 明确的输入等待模式（英文官方文案 + 中文输入等待类）
    const INPUT_WAIT_PATTERNS: [&str; 5] = [
        "waiting for your input",
        "等待输入",
        "等待您输入",
        "等待您的输入",
        "请输入",
    ];
    for pattern in INPUT_WAIT_PATTERNS {
        if lower.contains(pattern) {
            // 权限类文案明确不算等待输入（即使同时含等待字样）
            if lower.contains("permission") {
                return false;
            }
            return true;
        }
    }
    false
}

// truncate / is_system_marker / to_local_iso / short_time 定义在 utils.rs
// events_dir / claude_dir 定义在 config.rs（本模块只读，不定义路径）

/// 检测 hook 安装/注册状态（设置页展示）。
///
/// 返回 JSON：`{ installed, registered, broken, broken_reason }`
/// - installed：claude 目录下 `ccbuddy-hook[.exe]` 文件存在
/// - registered：`settings.json` 的 hooks.<事件> 下存在指向该 hook 的 command
/// - broken：状态异常（文件存在但 Unix 上无可执行位 / 注册的 command 指向的
///   hook 二进制不存在——常见于 claude_dir 变更后旧目录注册残留）
/// - broken_reason：broken 时的具体原因（正常为 None）
pub fn hook_status() -> serde_json::Value {
    let hook_name = crate::hook_file_name();
    let claude = claude_dir();
    let hook_path = claude.join(hook_name);
    let installed = hook_path.is_file();
    let missing_exec_bit = missing_exec_bit(&hook_path, installed);
    let registered = registered_hooks(&claude, &hook_path);

    // 一致性校验（安装与注册的相互对账）：
    // - 已注册但 hook 二进制不存在（claude_dir 变更/文件被删）→ 注册指向空目标
    // - 已安装但 Unix 上无可执行位 → Claude Code 无法 spawn
    // 二者都是"配置显示正常、实际不工作"的隐性故障，明确报出。
    let any_registered = registered.values().any(|v| v.as_bool() == Some(true));
    let broken = (any_registered && !installed) || missing_exec_bit;

    serde_json::json!({
        "installed": installed,
        "registered": registered,
        "broken": broken,
        "broken_reason": broken_reason(missing_exec_bit, any_registered, installed, &hook_path),
    })
}

/// hook 文件存在但没有可执行位（Unix 专有；Windows 无此概念，恒 false）。
#[cfg(unix)]
fn missing_exec_bit(hook_path: &Path, installed: bool) -> bool {
    use std::os::unix::fs::PermissionsExt;
    installed
        && std::fs::metadata(hook_path)
            .map(|m| m.permissions().mode() & 0o111 == 0)
            .unwrap_or(true)
}

#[cfg(not(unix))]
fn missing_exec_bit(_hook_path: &Path, _installed: bool) -> bool {
    false
}

/// 读 `settings.json`，逐个事件判断是否已注册本 hook。
///
/// 返回 `事件名 → 是否注册`；settings.json 缺失 / 损坏 / 无 hooks 字段时
/// 返回空 map（调用方按「未注册」展示）。
fn registered_hooks(claude: &Path, hook_path: &Path) -> serde_json::Map<String, serde_json::Value> {
    let root = read_settings_json(claude);

    // 注册在 settings.json 里的是路径字符串，Windows 反斜杠统一成斜杠后比对
    let command = hook_path.to_string_lossy().replace('\\', "/");
    let mut registered = serde_json::Map::new();
    let Some(hooks) = root.get("hooks").and_then(|h| h.as_object()) else {
        return registered;
    };
    for ev in crate::HOOK_EVENTS {
        // 该事件下是否存在指向 ccbuddy-hook 的 entry（兼容扁平/三层两种格式）
        let mut hit = false;
        if let Some(arr) = hooks.get(ev).and_then(|v| v.as_array()) {
            for entry in arr {
                if crate::entry_has_command(entry, &command) {
                    hit = true;
                    break;
                }
            }
        }
        registered.insert(ev.to_string(), serde_json::Value::Bool(hit));
    }
    registered
}

/// broken 时的具体原因（正常为 None）。判定顺序与 [`hook_status`] 里的
/// `broken` 一致：可执行位问题优先于「注册了但文件不在」。
fn broken_reason(
    missing_exec_bit: bool,
    any_registered: bool,
    installed: bool,
    hook_path: &Path,
) -> Option<String> {
    if missing_exec_bit {
        return Some(format!(
            "hook 文件存在但无可执行权限，请重新安装 hook（或 chmod +x {}）",
            hook_path.display()
        ));
    }
    if any_registered && !installed {
        return Some(format!(
            "settings.json 已注册 hook，但 {} 不存在（Claude 目录变更或文件被删），请在此目录重新安装",
            hook_path.display()
        ));
    }
    None
}

/// 从文件名 `event-<session_id>.jsonl` 提取 session_id。
///
/// 文件名非法（拿不到文件名）时返回空串，调用方以空串判定为"非会话文件"并跳过。
fn session_id_from_filename(path: &Path) -> String {
    let name: String = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    name.strip_prefix("event-")
        .unwrap_or_default() // 无 event- 前缀 → 空串（调用方跳过）
        .trim_end_matches(".jsonl")
        .to_string()
}

/// 事件流会话列表（hook 日志 `~/.ccbuddy/events`，实时会话，状态由状态机推断）。
///
/// 增量刷新：每个会话文件按 mtime 缓存解析结果，只有更新的文件才重新读取；
/// 每个会话只保留最新 [`MAX_EVENTS_PER_SESSION`] 条事件。
pub fn load_events() -> Vec<SessionInfo> {
    collect_events()
}

/// 收集并聚合事件流会话列表（`load_events` 的实现，watcher 增量探测复用本函数）。
fn collect_events() -> Vec<SessionInfo> {
    let dir = events_dir();
    // 目录不存在（hook 从未安装/运行过）：返回空列表
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };

    let mut cache = event_cache().lock().unwrap();
    let mut out = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (session_id, path) in session_event_files(rd) {
        seen.insert(session_id.clone());
        let Some((mtime, size)) = file_fingerprint(&path) else {
            continue;
        };
        out.push(resolve_session(&path, &session_id, mtime, size, &mut cache));
    }

    // 清理日志文件已删除的会话缓存，并同步删除对应快照（M-B10）
    cache.retain(|k, _| seen.contains(k));
    remove_stale_snapshots(&seen);

    // 时效降级：活跃状态超时无新事件 → idle（缓存放原始状态，此处按当前时间判定）
    for info in &mut out {
        downgrade_stale(info);
    }

    out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));

    // M-B10：会话列表产出后持久化状态快照（内容有差异才写盘，控制 IO 频率）
    persist_snapshots(&out);

    out
}

/// 从目录里挑出事件日志文件，返回 `(session_id, 路径)`。
///
/// 文件名不合规的条目（子目录、别的文件）按「非会话文件」跳过。
/// 只做筛选不做读取：文件读不出来是下一环的事，不影响 `seen` 的完整性。
fn session_event_files(rd: std::fs::ReadDir) -> Vec<(String, PathBuf)> {
    let mut files = Vec::new();
    for entry in rd.flatten() {
        let path = entry.path();
        let session_id = session_id_from_filename(&path);
        if session_id.is_empty() {
            continue;
        }
        files.push((session_id, path));
    }
    files
}

/// 取单个会话的最新聚合结果：缓存命中直接复用，否则重新解析并垫底，再写回缓存。
///
/// 垫底有两级，都只在「日志本身给不出」时生效：
/// - 内存缓存里的上一次结果（首条用户输入滚出尾部窗口时补标题 / cwd）；
/// - M-B10 状态快照（重启后缓存为空时补标题与空白会话的状态/活动时间）。
fn resolve_session(
    path: &Path,
    session_id: &str,
    mtime: SystemTime,
    size: u64,
    cache: &mut HashMap<String, CachedSession>,
) -> SessionInfo {
    // 文件未变化（mtime + size 都一致，M-B11）：直接复用上次解析结果
    if let Some(c) = cache.get(session_id) {
        if cache_hit(c, mtime, size) {
            return c.info.clone();
        }
    }

    let mut info = parse_event_file(path, session_id);
    // 只读尾部 N 条时，首条用户输入（标题来源）可能不在窗口内，沿用旧标题
    if let Some(prev) = cache.get(session_id) {
        if info.title == UNNAMED_SESSION && prev.info.title != UNNAMED_SESSION {
            info.title = prev.info.title.clone();
        }
        if info.cwd.is_empty() {
            info.cwd = prev.info.cwd.clone();
            info.project = prev.info.project.clone();
        }
    }
    // M-B10：日志能给出的信息一律以日志为准，快照只补标题 / 空白会话
    // （无有效事件）的状态与活动时间；补上的活跃状态仍在 collect 出口
    // 受超时降级约束。
    if let Some(snap) = read_snapshot(session_id) {
        apply_snapshot_fallback(&mut info, &snap);
    }
    cache.insert(
        session_id.to_string(),
        CachedSession {
            mtime,
            size,
            info: info.clone(),
        },
    );
    info
}

/// 历史会话列表与详情已迁移至 [`crate::claude`]（Claude 原生 transcript 解析模块）。

/// 紧急会话（等待确认 / 等待输入 / 出错）的 id 列表（任务栏通知用）。
#[cfg(feature = "gui")]
pub fn urgent_session_ids() -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for session in load_events() {
        let urgent = matches!(
            session.status.as_str(),
            "waiting_confirmation" | "waiting_input" | "error"
        );
        if urgent {
            out.push(session.id);
        }
    }
    out
}

/// 按需加载事件流会话详情（hook 日志 `~/.ccbuddy/events`，最新 50 条事件）。
pub fn load_event_detail(session_id: &str) -> Option<SessionInfo> {
    let path = events_dir().join(format!("event-{session_id}.jsonl"));
    if path.is_file() {
        let mut info = parse_event_file(&path, session_id);
        downgrade_stale(&mut info);
        Some(info)
    } else {
        None
    }
}

/// 文件指纹：修改时间 + 大小（M-B11：mtime 粒度 1-2 秒，同秒追加靠 size 探测）。
///
/// 返回 `(mtime, size)`；元数据读取失败返回 None（调用方跳过该文件）。
fn file_fingerprint(path: &Path) -> Option<(SystemTime, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.modified().ok()?, meta.len()))
}

/// 缓存是否可复用：mtime 与 size 都一致（M-B11）。
fn cache_hit(c: &CachedSession, mtime: SystemTime, size: u64) -> bool {
    c.mtime == mtime && c.size == size
}

// ---- 会话状态快照（M-B10）----
//
// 只解析日志尾部 50 条、解析缓存又在内存，重启即空：首条 UserPromptSubmit
// 滚出窗口后会话名回落「(未命名)」。这里把每个会话最后已知状态持久化到
// `~/.ccbuddy/state/<session-id>.json`（ccbuddy 自己的数据目录，不碰 ~/.claude/），
// 重启后作为垫底：日志解析为准，快照仅在日志信息不足时补。

/// 单个会话的最后已知状态快照（写盘格式，serde_json）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
struct SessionSnapshot {
    status: String,
    title: String,
    last_activity: String,
}

/// 快照目录：`~/.ccbuddy/state`。
fn state_dir() -> PathBuf {
    crate::config::data_root().join("state")
}

/// 读单个会话快照（文件缺失 / 损坏返回 None）。
fn read_snapshot_from(dir: &Path, session_id: &str) -> Option<SessionSnapshot> {
    let text = std::fs::read_to_string(dir.join(format!("{session_id}.json"))).ok()?;
    serde_json::from_str(&text).ok()
}

fn read_snapshot(session_id: &str) -> Option<SessionSnapshot> {
    read_snapshot_from(&state_dir(), session_id)
}

/// M-B10：日志解析信息不足时用快照垫底。
///
/// - 标题为「(未命名会话)」（首条用户输入滚出尾部窗口）→ 补标题；
/// - 窗口内没有任何有效事件（`last_activity` 为空）→ 补状态与活动时间，
///   补上的活跃状态仍会在 collect 出口受 [`downgrade_stale`] 超时降级约束。
///
/// 日志能给出的信息一律以日志为准，快照只补缺。
fn apply_snapshot_fallback(info: &mut SessionInfo, snap: &SessionSnapshot) {
    if info.title == UNNAMED_SESSION && !snap.title.is_empty() {
        info.title = snap.title.clone();
    }
    if info.last_activity.is_empty() && !snap.last_activity.is_empty() {
        info.status = snap.status.clone();
        info.last_activity = snap.last_activity.clone();
    }
}

/// 上次写出的快照内容（session_id → JSON 串）：内容有差异才写盘，控制 IO 频率。
fn last_written_snapshots() -> &'static Mutex<HashMap<String, String>> {
    static LAST: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    LAST.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 把会话列表的最后已知状态写入快照目录，返回本次实际写盘的文件数。
///
/// 写入失败仅 warn（快照是垫底优化，失败不影响功能）。`last` 由调用方传入
/// （生产用全局 [`last_written_snapshots`]，单测注入局部 Map 隔离）。
fn persist_snapshots_to(
    dir: &Path,
    sessions: &[SessionInfo],
    last: &mut HashMap<String, String>,
) -> usize {
    let mut written = 0;
    for s in sessions {
        let snap = SessionSnapshot {
            status: s.status.clone(),
            title: s.title.clone(),
            last_activity: s.last_activity.clone(),
        };
        let Ok(text) = serde_json::to_string(&snap) else {
            continue;
        };
        // 内容与上次一致：跳过写盘（watcher 每秒触发也不产生 IO）
        if last.get(&s.id).is_some_and(|t| t == &text) {
            continue;
        }
        let path = dir.join(format!("{}.json", s.id));
        if std::fs::create_dir_all(dir)
            .and_then(|_| std::fs::write(&path, &text))
            .is_err()
        {
            log::warn!("写入会话快照失败: {}", path.display());
            continue;
        }
        last.insert(s.id.clone(), text);
        written += 1;
    }
    written
}

fn persist_snapshots(sessions: &[SessionInfo]) {
    let mut last = last_written_snapshots().lock().unwrap();
    persist_snapshots_to(&state_dir(), sessions, &mut last);
}

/// 删除日志文件已消失的会话快照（与缓存 retain 同步，M-B10）。
///
/// 只删 `<session-id>.json`（快照目录是 ccbuddy 专属数据目录，仍按扩展名
/// 过滤以防误删他物）；删除失败仅 warn。返回本次删除的文件数。
fn remove_stale_snapshots_from(
    dir: &Path,
    seen: &HashSet<String>,
    last: &mut HashMap<String, String>,
) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Some(id) = path.file_stem().map(|s| s.to_string_lossy().to_string()) else {
            continue;
        };
        if seen.contains(&id) {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("删除会话快照失败 {}: {e}", path.display());
            continue;
        }
        last.remove(&id);
        removed += 1;
    }
    removed
}

fn remove_stale_snapshots(seen: &HashSet<String>) {
    let mut last = last_written_snapshots().lock().unwrap();
    remove_stale_snapshots_from(&state_dir(), seen, &mut last);
}

// ---- 事件日志清理（L-B15）----

/// 事件日志保留天数：超过该天数未更新的 event-*.jsonl 在主程序启动时清理。
const EVENT_LOG_RETENTION_DAYS: u64 = 30;

/// 启动清理：删除 `cutoff` 之前未更新的 event-*.jsonl（幂等，失败仅 warn）。
///
/// 只删 events 目录下文件名匹配 `event-<session-id>.jsonl` 的日志
/// （复用 [`session_id_from_filename`] 判定，其他文件不碰）。返回删除数。
fn cleanup_old_event_logs_before(dir: &Path, cutoff: SystemTime) -> usize {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in rd.flatten() {
        let path = entry.path();
        // 只认 event-*.jsonl（无该前缀的文件名返回空串）
        if session_id_from_filename(&path).is_empty() {
            continue;
        }
        let Ok(meta) = entry.metadata() else {
            continue;
        };
        let Ok(mtime) = meta.modified() else {
            continue;
        };
        if mtime >= cutoff {
            continue;
        }
        if let Err(e) = std::fs::remove_file(&path) {
            log::warn!("清理过期事件日志失败 {}: {e}", path.display());
            continue;
        }
        removed += 1;
    }
    removed
}

/// 主程序启动时调用一次（run_server 与 GUI setup）：清理超过
/// [`EVENT_LOG_RETENTION_DAYS`] 天未更新的 event-*.jsonl。幂等。
pub fn cleanup_old_event_logs() {
    let cutoff = SystemTime::now()
        - std::time::Duration::from_secs(EVENT_LOG_RETENTION_DAYS * 24 * 3600);
    cleanup_old_event_logs_before(&events_dir(), cutoff);
}

/// 解析单个事件日志文件 `event-<session_id>.jsonl`，聚合为会话信息。
///
/// 只读文件尾部（最新 [`MAX_EVENTS_PER_SESSION`] 条事件），大文件不必全量读入。
fn parse_event_file(path: &Path, session_id: &str) -> SessionInfo {
    let lines = read_tail_lines(path, MAX_EVENTS_PER_SESSION);
    let mut agg: Option<SessionAgg> = None;
    for line in &lines {
        let Some(ev) = Event::parse(line, session_id) else {
            continue;
        };
        // 首个事件创建聚合器（会话 id 优先取 payload 中的值，缺失时用文件名提取的 id）
        let a = agg.get_or_insert_with(|| SessionAgg::new(ev.session_id.clone()));
        a.apply(&ev);
    }
    match agg {
        Some(a) => a.into_info(),
        // 一条有效事件都没有：返回空白会话占位
        None => SessionAgg::new(session_id.to_string()).into_info(),
    }
}

/// 读取文件尾部的完整行（最多 `max_lines` 条）。
///
/// 大文件只读最后 512KB；尾部块内行数不足时（单行超长）回退全量读取。
fn read_tail_lines(path: &Path, max_lines: usize) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    const CHUNK: u64 = 512 * 1024;
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = f.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    // 小文件：全量读与读尾部等价
    if len <= CHUNK {
        return read_all_lines(path).unwrap_or_default();
    }

    // L-B16：seek 位置前一个字节是 \n → 尾部块首行本身是完整行，应保留；
    // 否则首行从行中间截断（可能截在多字节字符中间），照旧丢弃。
    let first_line_complete = prev_byte_is_newline(&mut f, len - CHUNK - 1);
    f.seek(SeekFrom::Start(len - CHUNK)).ok();
    let mut buf = Vec::new();
    f.take(CHUNK).read_to_end(&mut buf).ok();
    // 首行可能从多字节字符中间被截断，用 lossy 转换并丢弃首行
    let mut lines = split_nonempty_lines(&String::from_utf8_lossy(&buf));
    if !first_line_complete && !lines.is_empty() {
        lines.remove(0);
    }
    // 尾部块行数不足（首行超长等）：回退全量读取
    if lines.len() < max_lines {
        if let Some(all) = read_all_lines(path) {
            lines = all;
        }
    }
    lines
}

/// 全量读取并按行切分；读失败（含非 UTF-8）返回 None，调用方保留已有结果。
fn read_all_lines(path: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(split_nonempty_lines(&text))
}

/// 按行切分，丢掉空行与行首尾空白。
fn split_nonempty_lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
}

/// `pos` 处的字节是否为换行符（用来判断下一个字节是不是一行的开头）。
///
/// 读不到（seek 失败等）按「不是」保守处理：宁可按截断丢弃首行，
/// 也不要留着半行 JSON。
fn prev_byte_is_newline(f: &mut std::fs::File, pos: u64) -> bool {
    use std::io::{Read, Seek, SeekFrom};

    // f.take 会 move 句柄，探测用一个克隆
    let Ok(mut probe) = f.try_clone() else {
        return false;
    };
    if probe.seek(SeekFrom::Start(pos)).is_err() {
        return false;
    }
    let mut prev = [0u8; 1];
    match probe.read_exact(&mut prev) {
        Ok(()) => prev[0] == b'\n',
        Err(_) => false,
    }
}

// 原生 transcript 解析在 claude.rs

// ---- 事件变更推送（SSE / Tauri event 共用） ----

/// 变更通知回调：事件流有新数据时调用（含解析好的会话列表）。
pub type ChangeListener = Box<dyn Fn(Vec<SessionInfo>) + Send + Sync>;

/// 注销凭据（guard 语义见 [`crate::watch::ListenerGuard`]）。
pub type ListenerGuard = crate::watch::ListenerGuard<ChangeListener>;

/// 全局监听者集合：server 的 SSE broadcaster 与桌面端的通知都挂在这里。
static CHANGE_LISTENERS: crate::watch::ListenerRegistry<ChangeListener> =
    crate::watch::ListenerRegistry::new();

/// 事件流推送周期：2 秒（前端事件流视图的定时刷新节奏）。
const PUSH_PERIOD: std::time::Duration = std::time::Duration::from_secs(2);

/// 事件流推送器：每 [`PUSH_PERIOD`] 取一次数据推给监听者。
static WATCHER: crate::watch::PollingWatcher = crate::watch::PollingWatcher::new();

/// 启动事件流推送：每 [`PUSH_PERIOD`] 回调一次全部监听者。
///
/// 服务端（SSE）与桌面端（Tauri event）共享同一推送线程，同一批数据只解析一次；
/// 取数走 mtime 增量缓存，没有新事件时不会重复解析。
pub fn start_watcher() {
    WATCHER.start(|| PUSH_PERIOD, || CHANGE_LISTENERS.notify(collect_events()));
}

/// 注册一个变更监听者，事件流变化时收到解析好的会话列表。
pub fn subscribe(listener: ChangeListener) -> ListenerGuard {
    start_watcher();
    CHANGE_LISTENERS.subscribe(listener)
}


#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 工具概要优先取结构化字段，且必须限长（Write 类入参是整份文件内容）。
    #[test]
    fn tool_brief_prefers_key_and_is_bounded() {
        // 键序优先于字母序：content 排在前也不该被选中
        let input = serde_json::json!({"content": "a".repeat(5000), "file_path": "src/main.rs"});
        assert_eq!(tool_brief(&input), "src/main.rs");

        // 无结构化字段时回退紧凑 JSON，并截断到上限
        let long = serde_json::json!({"weird_key": "b".repeat(5000)});
        let brief = tool_brief(&long);
        assert_eq!(brief.chars().count(), TOOL_BRIEF_MAX + 1, "截断后含省略号");
        assert!(brief.ends_with('…'));

        // 空对象：不 panic，回退空 JSON 串
        assert_eq!(tool_brief(&serde_json::json!({})), "{}");
    }

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

        // 会话 A：PreToolUse 不再触发等待确认（每次工具调用前都触发），应为 running
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-a","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-a","cwd":"D:/work/proj","prompt":"帮我修复支付回调"}}"#);
        append(&dir, "sess-a", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PreToolUse","payload":{"session_id":"sess-a","cwd":"D:/work/proj","tool_name":"Bash"}}"#);

        // 会话 B：一轮正常结束（Stop 收尾），应为 completed
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-b","cwd":"D:/work/other"}}"#);
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-b","cwd":"D:/work/other","prompt":"生成文档"}}"#);
        append(&dir, "sess-b", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"Stop","payload":{"session_id":"sess-b","cwd":"D:/work/other"}}"#);

        // 会话 C：权限弹窗（PermissionRequest 驱动等待确认）
        append(&dir, "sess-c", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-c","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-c", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-c","cwd":"D:/work/proj","prompt":"部署到测试环境"}}"#);
        append(&dir, "sess-c", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PreToolUse","payload":{"session_id":"sess-c","cwd":"D:/work/proj","tool_name":"Bash"}}"#);
        append(&dir, "sess-c", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"PermissionRequest","payload":{"session_id":"sess-c","cwd":"D:/work/proj","tool_name":"Bash","reason":"Bash command blocked by settings"}}"#);

        let sessions: Vec<SessionInfo> = ["sess-a", "sess-b", "sess-c"]
            .iter()
            .map(|id| parse_event_file(&dir.join(format!("event-{id}.jsonl")), id))
            .collect();

        assert_eq!(sessions.len(), 3);
        let a = sessions.iter().find(|s| s.id == "sess-a").unwrap();
        let b = sessions.iter().find(|s| s.id == "sess-b").unwrap();
        let c = sessions.iter().find(|s| s.id == "sess-c").unwrap();

        // H-B1：PreToolUse → Running（不再置等待确认）
        assert_eq!(a.status, "running");
        assert_eq!(a.title, "帮我修复支付回调");
        assert_eq!(a.project, "proj");
        assert!(!a.unread);

        // H-B3：Stop → Completed
        assert_eq!(b.status, "completed");
        assert_eq!(b.title, "生成文档");
        assert_eq!(b.project, "other");
        assert!(!b.unread);

        // H-B2：PermissionRequest → WaitingConfirmation（unread 紧急状态）
        assert_eq!(c.status, "waiting_confirmation");
        assert!(c.unread);
        // 权限请求同时作为 system 消息展示
        assert!(
            c.messages
                .iter()
                .any(|m| m.msg_type == "system" && m.content.contains("请求权限") && m.content.contains("Bash"))
        );

        // 消息应包含用户提示词和工具调用
        assert!(a.messages.iter().any(|m| m.msg_type == "user"));
        assert!(a.messages.iter().any(|m| m.tool_call.as_deref() == Some("Bash")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-B2：PermissionDenied / PostToolUse 清除 WaitingConfirmation，且 Stop 后不卡死。
    #[test]
    fn permission_denied_clears_waiting_confirmation() {
        let dir = std::env::temp_dir().join("ccbuddy-test-perm-denied");
        let _ = std::fs::remove_dir_all(&dir);

        // 场景 1：用户拒绝权限 → PermissionDenied 复位到 Running
        append(&dir, "sess-deny", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-deny","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-deny", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-deny","prompt":"部署"}}"#);
        append(&dir, "sess-deny", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PermissionRequest","payload":{"session_id":"sess-deny","tool_name":"Bash"}}"#);
        append(&dir, "sess-deny", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"PermissionDenied","payload":{"session_id":"sess-deny","tool_name":"Bash"}}"#);

        // 场景 2：用户批准权限 → PostToolUse 清除等待确认，随后 Stop 收尾
        append(&dir, "sess-allow", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-allow","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-allow", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-allow","prompt":"执行测试"}}"#);
        append(&dir, "sess-allow", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PermissionRequest","payload":{"session_id":"sess-allow","tool_name":"Bash"}}"#);
        append(&dir, "sess-allow", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"PostToolUse","payload":{"session_id":"sess-allow","tool_name":"Bash"}}"#);
        append(&dir, "sess-allow", r#"{"received_at":"2026-08-20T14:04:00Z","hook_event":"Stop","payload":{"session_id":"sess-allow"}}"#);

        let deny = parse_event_file(&dir.join("event-sess-deny.jsonl"), "sess-deny");
        let allow = parse_event_file(&dir.join("event-sess-allow.jsonl"), "sess-allow");

        assert_eq!(deny.status, "running", "PermissionDenied 后不应停留在等待确认");
        assert!(!deny.unread);
        assert!(deny.messages.iter().any(|m| m.content.contains("权限被拒")));

        assert_eq!(allow.status, "completed", "批准后 PostToolUse→Stop 应为 completed");
        assert!(!allow.unread);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-B4：活跃状态超时无新事件 → 降级 idle；近期事件不降级；时间解析失败不降级。
    #[test]
    fn stale_active_sessions_downgrade_to_idle() {
        let mut recent = SessionInfo {
            id: "recent".into(),
            project: "proj".into(),
            cwd: "D:/work/proj".into(),
            title: "t".into(),
            status: "running".into(),
            last_activity: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S%.3f%:z").to_string(),
            preview: String::new(),
            unread: false,
            messages: Vec::new(),
            transcript_path: None,
            source: None,
            subagents: Vec::new(),
        };
        downgrade_stale(&mut recent);
        assert_eq!(recent.status, "running", "10 分钟内的活跃会话不降级");

        let mut stale_running = recent.clone();
        stale_running.id = "stale-running".into();
        stale_running.last_activity = (chrono::Local::now() - chrono::Duration::seconds(STALE_ACTIVE_AFTER_SECS + 60))
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string();
        downgrade_stale(&mut stale_running);
        assert_eq!(stale_running.status, "idle", "超时 running 应降级 idle");

        let mut stale_confirm = recent.clone();
        stale_confirm.id = "stale-confirm".into();
        stale_confirm.status = "waiting_confirmation".into();
        stale_confirm.unread = true;
        stale_confirm.last_activity = stale_running.last_activity.clone();
        downgrade_stale(&mut stale_confirm);
        assert_eq!(stale_confirm.status, "idle", "超时 waiting_confirmation 应降级 idle");
        assert!(!stale_confirm.unread, "降级后不再是紧急会话");

        let mut stale_input = recent.clone();
        stale_input.id = "stale-input".into();
        stale_input.status = "waiting_input".into();
        stale_input.last_activity = stale_running.last_activity.clone();
        downgrade_stale(&mut stale_input);
        assert_eq!(stale_input.status, "idle", "超时 waiting_input 应降级 idle");

        // 非活跃状态不受降级影响
        let mut completed = recent.clone();
        completed.id = "completed".into();
        completed.status = "completed".into();
        completed.last_activity = stale_running.last_activity.clone();
        downgrade_stale(&mut completed);
        assert_eq!(completed.status, "completed", "completed 不受时效降级影响");

        // 时间解析失败：宁可不降级也不误降级
        let mut bad_time = recent.clone();
        bad_time.id = "bad-time".into();
        bad_time.last_activity = "not-a-timestamp".into();
        downgrade_stale(&mut bad_time);
        assert_eq!(bad_time.status, "running", "时间解析失败不降级");
    }

    #[test]
    fn parses_failure_and_meta_events() {
        let dir = std::env::temp_dir().join("ccbuddy-test-new-events");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-fail","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-fail","prompt":"编译项目"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"PostToolUseFailure","payload":{"session_id":"sess-fail","tool_name":"Bash"}}"#);
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"FileChanged","payload":{"session_id":"sess-fail","file_path":"D:/work/proj/.envrc"}}"#);
        // 出错后 Stop 收尾：has_error=true，Stop 不应把错误覆盖为 completed
        append(&dir, "sess-fail", r#"{"received_at":"2026-08-20T14:04:00Z","hook_event":"Stop","payload":{"session_id":"sess-fail"}}"#);

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
            r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"MessageDisplay","payload":{"session_id":"s","message":"hi"}}"#,
            "fallback",
        )
        .unwrap();
        assert_eq!(meta_description(&ev3), None);
    }

    /// M-B8：官方英文通知文案不误判；明确的输入等待文案才命中。
    #[test]
    fn notification_input_heuristics() {
        // 官方权限通知原文（含 "need"）：不算等待输入（那是等待确认，
        // 由 PermissionRequest 事件负责）
        assert!(!is_input_request(
            "Claude needs your permission to use Bash"
        ));
        assert!(!is_input_request(
            "Claude is waiting for your permission to use Bash"
        ));
        // 官方输入等待文案：命中
        assert!(is_input_request("Claude is waiting for your input"));
        // 中文输入等待类：命中
        assert!(is_input_request("Claude 正在等待输入"));
        assert!(is_input_request("Claude 正在等待您的输入"));
        assert!(is_input_request("请输入要搜索的关键词"));
        // 宽泛词（"需要" / "请"单独出现）不再触发：宁可漏判也不误判
        assert!(!is_input_request("Claude needs more context"));
        assert!(!is_input_request("Ready for the next task, please advise."));
        // 空 / 无关消息：不命中
        assert!(!is_input_request(""));
        assert!(!is_input_request("Task completed successfully"));
    }

    /// M-B9：带 parent_tool_use_id 的事件完全绕过状态机：不改变状态、
    /// 不更新 preview / title，只追加一条子代理活动 system 消息。
    #[test]
    fn subagent_events_do_not_touch_main_state() {
        let dir = std::env::temp_dir().join("ccbuddy-test-subagent");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, "sess-sub", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-sub","cwd":"D:/work/proj"}}"#);
        append(&dir, "sess-sub", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-sub","prompt":"调研子代理用法"}}"#);
        append(&dir, "sess-sub", r#"{"received_at":"2026-08-20T14:02:00Z","hook_event":"Stop","payload":{"session_id":"sess-sub"}}"#);
        // 子代理的 PreToolUse（payload 顶层带 parent_tool_use_id）：
        // 第一批后 PreToolUse→Running，若未隔离会把 completed 翻回 running 并更新 preview
        append(&dir, "sess-sub", r#"{"received_at":"2026-08-20T14:03:00Z","hook_event":"PreToolUse","payload":{"session_id":"sess-sub","cwd":"D:/work/proj","tool_name":"WebFetch","parent_tool_use_id":"toolu-abc"}}"#);

        let info = parse_event_file(&dir.join("event-sess-sub.jsonl"), "sess-sub");
        assert_eq!(info.status, "completed", "子代理事件不应改变主会话状态");
        assert_eq!(info.title, "调研子代理用法");
        // preview 保持在 Stop 前的展示（未被子代理工具调用覆盖）
        assert!(!info.preview.contains("WebFetch"), "子代理事件不应更新 preview");
        // 子代理活动只作为时间线 system 消息展示
        assert!(
            info.messages
                .iter()
                .any(|m| m.msg_type == "system" && m.content.contains("子代理活动") && m.content.contains("WebFetch"))
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-B13：SessionEnd 按 reason 区分：clear → Idle + system 消息；
    /// logout / 缺失 → Completed。
    #[test]
    fn session_end_distinguishes_reason() {
        let dir = std::env::temp_dir().join("ccbuddy-test-session-end");
        let _ = std::fs::remove_dir_all(&dir);

        // 场景 1：reason=clear（payload 顶层）→ Idle + 「上下文已清空」system 消息
        append(&dir, "sess-clear", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-clear","prompt":"清空上下文"}}"#);
        append(&dir, "sess-clear", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"SessionEnd","payload":{"session_id":"sess-clear","reason":"clear"}}"#);
        let clear = parse_event_file(&dir.join("event-sess-clear.jsonl"), "sess-clear");
        assert_eq!(clear.status, "idle", "clear 后会话仍会继续，应置 Idle");
        assert!(clear.messages.iter().any(|m| m.content.contains("上下文已清空")));

        // 场景 2：reason=logout → Completed
        append(&dir, "sess-out", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-out","prompt":"退出登录"}}"#);
        append(&dir, "sess-out", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"SessionEnd","payload":{"session_id":"sess-out","reason":"logout"}}"#);
        let logout = parse_event_file(&dir.join("event-sess-out.jsonl"), "sess-out");
        assert_eq!(logout.status, "completed", "logout 是真正结束，应为 Completed");

        // 场景 3：reason 缺失 → 维持原 Completed 逻辑
        append(&dir, "sess-none", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"UserPromptSubmit","payload":{"session_id":"sess-none","prompt":"结束"}}"#);
        append(&dir, "sess-none", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"SessionEnd","payload":{"session_id":"sess-none"}}"#);
        let none = parse_event_file(&dir.join("event-sess-none.jsonl"), "sess-none");
        assert_eq!(none.status, "completed", "reason 缺失维持原有 Completed 逻辑");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-B12：transcript_path / source 提取并进入 SessionInfo（payload 顶层）。
    #[test]
    fn extracts_transcript_path_and_source() {
        let dir = std::env::temp_dir().join("ccbuddy-test-toplevel-fields");
        let _ = std::fs::remove_dir_all(&dir);

        append(&dir, "sess-tp", r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"SessionStart","payload":{"session_id":"sess-tp","cwd":"D:/work/proj","source":"resume","transcript_path":"C:/users/x/.claude/projects/p/sess-tp.jsonl"}}"#);
        append(&dir, "sess-tp", r#"{"received_at":"2026-08-20T14:01:00Z","hook_event":"PreToolUse","payload":{"session_id":"sess-tp","tool_name":"Bash","transcript_path":"C:/users/x/.claude/projects/p/sess-tp.jsonl"}}"#);

        let info = parse_event_file(&dir.join("event-sess-tp.jsonl"), "sess-tp");
        assert_eq!(
            info.transcript_path.as_deref(),
            Some("C:/users/x/.claude/projects/p/sess-tp.jsonl")
        );
        assert_eq!(info.source.as_deref(), Some("resume"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-B10：快照写读 roundtrip + 内容有差异才写盘。
    #[test]
    fn snapshot_persist_roundtrip_and_skip_unchanged() {
        let dir = std::env::temp_dir().join("ccbuddy-test-snapshot");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let mut last: HashMap<String, String> = HashMap::new();
        let sessions = vec![SessionInfo {
            id: "s1".into(),
            project: "proj".into(),
            cwd: "D:/work/proj".into(),
            title: "修复登录".into(),
            status: "running".into(),
            last_activity: "2026-09-15T10:00:00.000+08:00".into(),
            preview: String::new(),
            unread: false,
            messages: Vec::new(),
            transcript_path: None,
            source: None,
            subagents: Vec::new(),
        }];

        // 首次：写盘 1 个文件，读回内容一致
        assert_eq!(persist_snapshots_to(&dir, &sessions, &mut last), 1);
        let snap = read_snapshot_from(&dir, "s1").unwrap();
        assert_eq!(snap.status, "running");
        assert_eq!(snap.title, "修复登录");
        assert_eq!(snap.last_activity, "2026-09-15T10:00:00.000+08:00");

        // 内容未变：跳过写盘（last 与盘上一致）
        assert_eq!(persist_snapshots_to(&dir, &sessions, &mut last), 0);

        // 状态变化：重新写盘
        let mut changed = sessions.clone();
        changed[0].status = "completed".into();
        assert_eq!(persist_snapshots_to(&dir, &changed, &mut last), 1);
        assert_eq!(read_snapshot_from(&dir, "s1").unwrap().status, "completed");

        // 会话消失：快照同步删除
        let seen: HashSet<String> = HashSet::new();
        let mut last2: HashMap<String, String> = HashMap::new();
        assert_eq!(remove_stale_snapshots_from(&dir, &seen, &mut last2), 1);
        assert!(read_snapshot_from(&dir, "s1").is_none());

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-B10：日志信息不足时用快照垫底（标题补全、空白会话补状态）。
    #[test]
    fn snapshot_fills_missing_title_and_blank_session() {
        // 标题滚出窗口 → 「(未命名会话)」时补标题
        let mut info = SessionInfo {
            id: "s2".into(),
            project: "proj".into(),
            cwd: String::new(),
            title: "(未命名会话)".into(),
            status: "running".into(),
            last_activity: "2026-09-15T10:00:00.000+08:00".into(),
            preview: String::new(),
            unread: false,
            messages: Vec::new(),
            transcript_path: None,
            source: None,
            subagents: Vec::new(),
        };
        let snap = SessionSnapshot {
            status: "completed".into(),
            title: "旧标题".into(),
            last_activity: "2026-09-15T09:00:00.000+08:00".into(),
        };
        apply_snapshot_fallback(&mut info, &snap);
        assert_eq!(info.title, "旧标题", "日志标题缺失时补快照标题");
        // 日志已有信息（状态 / 活动时间）不被快照覆盖
        assert_eq!(info.status, "running");
        assert_eq!(info.last_activity, "2026-09-15T10:00:00.000+08:00");

        // 空白会话（无有效事件）：状态与活动时间都补快照
        let mut blank = SessionInfo {
            id: "s3".into(),
            project: "proj".into(),
            cwd: String::new(),
            title: "(未命名会话)".into(),
            status: "idle".into(),
            last_activity: String::new(),
            preview: String::new(),
            unread: false,
            messages: Vec::new(),
            transcript_path: None,
            source: None,
            subagents: Vec::new(),
        };
        apply_snapshot_fallback(&mut blank, &snap);
        assert_eq!(blank.title, "旧标题");
        assert_eq!(blank.status, "completed");
        assert_eq!(blank.last_activity, "2026-09-15T09:00:00.000+08:00");

        // 快照标题为空：不把「(未命名会话)」换成空串
        let mut info2 = blank.clone();
        info2.title = "(未命名会话)".into();
        info2.last_activity = "2026-09-15T10:00:00.000+08:00".into();
        let empty_snap = SessionSnapshot {
            status: "running".into(),
            title: String::new(),
            last_activity: "2026-09-15T09:00:00.000+08:00".into(),
        };
        apply_snapshot_fallback(&mut info2, &empty_snap);
        assert_eq!(info2.title, "(未命名会话)");
    }

    /// L-B15：清理过期 event-*.jsonl；新文件与其他文件不动。
    #[test]
    fn cleanup_removes_only_old_event_logs() {
        let dir = std::env::temp_dir().join("ccbuddy-test-cleanup");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let old = dir.join("event-sess-old.jsonl");
        std::fs::write(&old, "{}\n").unwrap();
        let now = SystemTime::now();
        // 注：测试环境无法回写文件 mtime（Windows 需 set_file_time API），
        // 用两个 cutoff 方向验证判定逻辑：cutoff 在 30 天前 → 现存新文件不删；
        // cutoff 在未来（等价所有现存文件都"过期"）→ 只删 event-*.jsonl。

        // 新文件：cutoff 在 30 天前 → 不删
        let cutoff_old = now - std::time::Duration::from_secs(EVENT_LOG_RETENTION_DAYS * 24 * 3600);
        assert_eq!(cleanup_old_event_logs_before(&dir, cutoff_old), 0);
        assert!(old.is_file(), "30 天内更新的日志不应被清理");

        // 非事件日志文件混入：任何 cutoff 下都不删
        let other = dir.join("notes.txt");
        std::fs::write(&other, "keep me").unwrap();

        // cutoff 设到未来（模拟全部现存文件都"过期"）：只删 event-*.jsonl
        let cutoff_future = now + std::time::Duration::from_secs(1);
        assert_eq!(cleanup_old_event_logs_before(&dir, cutoff_future), 1);
        assert!(!old.exists(), "过期 event-*.jsonl 应被删除");
        assert!(other.is_file(), "非事件日志文件不应被误删");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-B11：同 mtime 不同 size → 缓存判定视为已变化（重读）。
    #[test]
    fn cache_hit_requires_same_mtime_and_size() {
        let t = SystemTime::UNIX_EPOCH;
        let c = CachedSession {
            mtime: t,
            size: 100,
            info: SessionInfo {
                id: "s".into(),
                project: "p".into(),
                cwd: String::new(),
                title: "t".into(),
                status: "running".into(),
                last_activity: String::new(),
                preview: String::new(),
                unread: false,
                messages: Vec::new(),
                transcript_path: None,
                source: None,
                subagents: Vec::new(),
            },
        };
        assert!(cache_hit(&c, t, 100), "mtime 与 size 都一致 → 命中");
        assert!(!cache_hit(&c, t, 200), "同 mtime 但 size 变化 → 不命中（重读）");
        assert!(
            !cache_hit(&c, t + std::time::Duration::from_secs(1), 100),
            "mtime 变化 → 不命中"
        );
    }

    /// L-B16：尾部块首行是完整行（seek 位置前一字节是 \n）时保留。
    #[test]
    fn read_tail_keeps_complete_first_line() {
        let dir = std::env::temp_dir().join("ccbuddy-test-tail");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 每行定长 512 字节（511 字符 + \n），总长为 512 的倍数：
        // seek 位置（len - 512KB）精确落在行边界上，前一字节是 \n → 首行完整。
        // 1031 行 × 512B = 527872B > 512KB，触发尾部块路径。
        let pad_line = "x".repeat(511);
        let json = r#"{"received_at":"2026-08-20T14:00:00Z","hook_event":"Stop","payload":{"session_id":"tail"}}"#;
        let last_line = format!("{json:<511}");
        assert_eq!(last_line.len(), 511);

        let path = dir.join("event-tail.jsonl");
        let mut f = std::fs::File::create(&path).unwrap();
        for _ in 0..1030 {
            writeln!(f, "{pad_line}").unwrap();
        }
        writeln!(f, "{last_line}").unwrap();
        drop(f);

        let lines = read_tail_lines(&path, MAX_EVENTS_PER_SESSION);
        assert!(lines.len() >= 2, "尾部块应至少包含填充行与最后事件行");
        // 首行在行边界上：完整保留，不以半行开头（collect 已 trim）
        assert_eq!(lines[0], pad_line, "行边界上的首行应完整保留");
        assert!(
            lines.last().map(|l| l.contains("Stop")).unwrap_or(false),
            "最后一行事件不应丢失"
        );

        // 对照：seek 点落在行中间时首行被截断（前一字节不是 \n）→ 丢弃，
        // 行数不足 max_lines 触发回退全量读取，完整长行被找回（旧有行为）
        let path2 = dir.join("event-tail2.jsonl");
        let mut f = std::fs::File::create(&path2).unwrap();
        let huge = "y".repeat(600 * 1024);
        writeln!(f, "{huge}").unwrap();
        writeln!(f, r#"{{"received_at":"2026-08-20T14:00:00Z","hook_event":"Stop","payload":{{"session_id":"tail2"}}}}"#).unwrap();
        drop(f);
        let lines2 = read_tail_lines(&path2, MAX_EVENTS_PER_SESSION);
        assert!(
            lines2.iter().any(|l| l == &huge),
            "回退全量读取应找回完整行"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-B14：注销 A 不影响 B 的回调（按稳定 id 移除，不错位）。
    #[test]
    fn listener_unregister_does_not_affect_others() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let a_calls = Arc::new(AtomicUsize::new(0));
        let b_calls = Arc::new(AtomicUsize::new(0));

        // 先注册 A，再注册 B；先注销 A，B 的回调必须仍然存活且被调用
        let ga = {
            let a = a_calls.clone();
            subscribe(Box::new(move |_| {
                a.fetch_add(1, Ordering::SeqCst);
            }))
        };
        let gb = {
            let b = b_calls.clone();
            subscribe(Box::new(move |_| {
                b.fetch_add(1, Ordering::SeqCst);
            }))
        };

        drop(ga); // 注销 A（按下标 remove 的旧实现会错位移除 B）

        CHANGE_LISTENERS.notify(Vec::new());
        assert_eq!(a_calls.load(Ordering::SeqCst), 0, "A 已注销，回调不应再被调用");
        assert_eq!(b_calls.load(Ordering::SeqCst), 1, "B 的回调应存活且被调用一次");

        drop(gb);
        CHANGE_LISTENERS.notify(Vec::new());
        assert_eq!(b_calls.load(Ordering::SeqCst), 1, "B 注销后回调也不应再被调用");
    }
}
