//! Claude Code 原生会话管理：读取 `~/.claude/projects/` 下的 transcript（只读）。
//!
//! 本模块只解析 Claude Code
//! 自己落盘的会话记录（`<claude_dir>/projects/<项目路径编码>/<session-id>.jsonl`），
//! 以聊天记录形式回放完整对话。
//!
//! 性能设计（会话很多时避免打开软件卡顿）：
//! - 所有会话的概要信息常驻内存：按 `(文件, mtime)` 缓存，启动时后台预热
//!   （[`prewarm`]/[`prewarm_async`]），之后 `load_sessions` 直接命中内存，
//!   只有新增/变化的 transcript 文件才重新解析；
//! - 概要解析（lazy 模式）只读文件头尾采样，不收集消息；
//! - 消息详情不占常驻内存：用户点开会话后 `load_session_detail` 才全量解析。
//!
//! Claude 目录用户可配置（`~/.ccbuddy/config.json`），默认 `~/.claude`。
//! 共享的展示模型与工具函数见 [`crate::utils`]。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::utils::{
    for_each_json_line, injected_user_line, is_system_marker, project_name_from_cwd,
    read_settings_json, short_time, to_local_iso, truncate, Message, SessionInfo, SubagentInfo,
    UNNAMED_SESSION, UNKNOWN_PROJECT,
};

/// 概要采样时读取的头部字节数：标题（首条用户输入 / custom-title）几乎总在文件头部。
const SUMMARY_HEAD_BYTES: u64 = 16 * 1024;
/// 概要采样时读取的尾部字节数：最新预览与最后活动时间在文件尾部。
const SUMMARY_TAIL_BYTES: u64 = 16 * 1024;
/// L-C7：头窗口解析不到内容（超长首行把 16KB 全占掉）时的倍增上限。
const SUMMARY_HEAD_MAX_BYTES: u64 = 256 * 1024;

/// 单个会话的概要缓存条目：文件未变化（mtime 相同）时直接复用，
/// 扫描列表只重新解析新增/有更新的 transcript 文件。
struct SummaryEntry {
    mtime: SystemTime,
    info: SessionInfo,
}

/// 概要缓存：session_id → 概要信息。启动时预热，之后仅增量更新。
fn summary_cache() -> &'static Mutex<HashMap<String, SummaryEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, SummaryEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 历史会话列表（Claude Code 原生 transcript，`<claude_dir>/projects/`）。
///
/// 历史界面只取原生数据，不混合 hook 日志。
/// 概要常驻内存：mtime 未变化的文件直接命中缓存，扫描本身只做目录枚举。
pub fn load_sessions() -> Vec<SessionInfo> {
    let mut out = scan_and_refresh();
    out.sort_by(|a, b| b.last_activity.cmp(&a.last_activity));
    out
}

/// 启动预热：后台线程解析所有历史会话概要，填充内存缓存。
/// 阻塞但只做一次；调用方应放到后台线程（[`prewarm_async`]）。
pub fn prewarm() {
    let _ = scan_and_refresh();
}

/// 异步预热：在独立后台线程填充概要缓存，不阻塞调用方。
pub fn prewarm_async() {
    // 线程创建失败（资源耗尽等）只记日志：预热是优化项，失败不影响功能
    if let Err(e) = std::thread::Builder::new()
        .name("claude-mgr-prewarm".into())
        .spawn(prewarm)
    {
        log::warn!("概要缓存预热线程启动失败: {e}");
    }
}

/// 扫描 projects 目录并增量刷新概要缓存，返回全部会话的概要列表。
///
/// 增量策略：mtime 与缓存一致 → 复用；不一致或新增 → 头尾采样重解析；
/// 文件已删除 → 从缓存清理。
fn scan_and_refresh() -> Vec<SessionInfo> {
    // 先枚举全部主会话 transcript（子代理转录不进历史会话列表）
    let files: Vec<(PathBuf, String, SystemTime)> = scan_transcripts(&projects_dir(), false)
        .into_iter()
        .map(|t| (t.path, t.session_id, t.mtime))
        .collect();

    // M-C6：同名 <session-id>.jsonl 只保留 mtime 最新的一条（不同项目
    // 目录可能出现同 id 文件；最新活动者即当前会话正在写的记录）
    let deduped = dedup_latest_by_id(files);

    let mut cache = summary_cache().lock().unwrap();
    let mut out = Vec::with_capacity(deduped.len());
    let mut seen: HashSet<String> = HashSet::new();

    for (path, id, mtime) in deduped {
        seen.insert(id.clone());
        let info = match cache.get(&id) {
            Some(e) if e.mtime == mtime => e.info.clone(),
            _ => {
                let info = parse_summary_head_tail(&path);
                cache.insert(id.clone(), SummaryEntry { mtime, info: info.clone() });
                info
            }
        };
        out.push(info);
    }

    // 清理 transcript 文件已删除的会话缓存
    cache.retain(|k, _| seen.contains(k));

    out
}

/// Claude Code 原生会话目录：`~/.claude/projects/<项目路径编码>/<session-id>.jsonl`。
/// 用量解析（[`crate::usage_mgr`]）复用同一目录扫描入口。
pub(crate) fn projects_dir() -> PathBuf {
    crate::config::claude_dir().join("projects")
}

/// 扫描结果的一条：一个 transcript 文件及其归属。
pub(crate) struct TranscriptFile {
    /// 文件路径。
    pub path: PathBuf,
    /// 会话 id（文件名派生：主会话是 `<id>`，子代理是 `agent-<id>`）。
    pub session_id: String,
    pub mtime: SystemTime,
    /// 文件大小（用量缓存的增量判据之一，M-B11）。
    pub size: u64,
    /// 宿主会话 id：子代理转录为其所在的 `<会话 id>/` 目录名，主会话为 `None`。
    /// 也是「是否为子代理」的判据（`Some` 即子代理）。
    pub owner_session: Option<String>,
}

/// 扫描 `projects/` 下的 transcript 文件。
///
/// - `include_subagents = false`：只要 `<项目>/<会话 id>.jsonl`（历史会话列表用）
/// - `include_subagents = true`：连 `<项目>/<会话 id>/subagents/agent-*.jsonl`
///   一起枚举（用量统计要算子代理的 token，H-C2）
///
/// 任何读取失败（权限 / 竞态删除）都只跳过该条目，不中断整轮扫描：
/// 扫描是一次尽力而为的快照，调用方据此按「有什么用什么」处理。
pub(crate) fn scan_transcripts(dir: &Path, include_subagents: bool) -> Vec<TranscriptFile> {
    let mut out: Vec<TranscriptFile> = Vec::new();
    // 目录不存在（Claude Code 尚未使用过）→ 空列表
    let Ok(project_dirs) = std::fs::read_dir(dir) else {
        return out;
    };
    for project_dir in project_dirs.flatten() {
        let project_path = project_dir.path();
        if !project_path.is_dir() {
            continue;
        }
        // 单个项目目录读取失败（权限等）：跳过该项目
        let Ok(entries) = std::fs::read_dir(&project_path) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                // 目录：只可能是 `<会话 id>/`，看它的 subagents/ 子目录
                if include_subagents {
                    scan_subagents_in(&path, &mut out);
                }
                continue;
            }
            if let Some(t) = transcript_entry(&path, None) {
                out.push(t);
            }
        }
    }
    out
}

/// 枚举 `<会话 id>/subagents/` 下的子代理转录。
fn scan_subagents_in(session_dir: &Path, out: &mut Vec<TranscriptFile>) {
    let owner = session_dir
        .file_name()
        .map(|n| n.to_string_lossy().to_string());
    let Ok(files) = std::fs::read_dir(session_dir.join("subagents")) else {
        return; // 无 subagents 目录（绝大多数会话）
    };
    for f in files.flatten() {
        if let Some(t) = transcript_entry(&f.path(), owner.clone()) {
            out.push(t);
        }
    }
}

/// 单个路径 → [`TranscriptFile`]：只认 `.jsonl`；拿不到文件名或元数据（竞态
/// 删除等）返回 `None`（调用方跳过）。
fn transcript_entry(path: &Path, owner_session: Option<String>) -> Option<TranscriptFile> {
    if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
        return None;
    }
    let session_id = path.file_stem()?.to_string_lossy().to_string();
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?;
    Some(TranscriptFile {
        path: path.to_path_buf(),
        session_id,
        mtime,
        size: meta.len(),
        owner_session,
    })
}

/// L-C8：读取 Claude Code 官方的 transcript 保留期（只读 ~/.claude/settings.json）。
///
/// Claude Code 按 `cleanupPeriodDays` 自动清理更早的 transcript（官方默认 30 天），
/// 历史会话列表据此提示用户「更早的会话已被自动清理」。读不到该配置项
/// （未设置 / 文件不存在）返回 None，前端不显示提示。
pub fn transcript_retention_days() -> Option<u32> {
    read_settings_json(&crate::config::claude_dir())
        .get("cleanupPeriodDays")
        .and_then(|x| x.as_u64())
        .map(|d| d as u32)
}

/// 同 id 多文件时按 mtime 最新去重（M-C6 逻辑核心，纯函数便于单测）。
///
/// 输入 `(路径, id, mtime)` 列表，每个 id 只保留 mtime 最大的那个条目，
/// 输出保持输入顺序（首个出现的最新条目位置）。
fn dedup_latest_by_id(files: Vec<(PathBuf, String, SystemTime)>) -> Vec<(PathBuf, String, SystemTime)> {
    // id → (mtime, 路径)：两遍扫描，先找每个 id 的最新条目（克隆路径断开借用）
    let mut best: HashMap<String, (SystemTime, PathBuf)> = HashMap::new();
    for (path, id, mtime) in &files {
        match best.get(id) {
            Some((t, _)) if *t >= *mtime => {}
            _ => {
                best.insert(id.clone(), (*mtime, path.clone()));
            }
        }
    }
    files
        .into_iter()
        .filter(|(path, id, mtime)| {
            best.get(id).is_some_and(|(t, p)| *t == *mtime && p == path)
        })
        .collect()
}

/// 在 `~/.claude/projects/` 下查找会话对应的 transcript 文件路径。
///
/// M-C6：不同项目目录下可能出现同名 `<session-id>.jsonl`（如 fork / 复制
/// 的会话记录）；命中多个时取 mtime 最新的那个（最新活动的文件即当前
/// 会话正在写的记录），不再依赖目录枚举顺序。
fn native_session_path(session_id: &str) -> Option<PathBuf> {
    // 命中多个（不同项目目录下的同名文件）时取 mtime 最新的那个
    scan_transcripts(&projects_dir(), false)
        .into_iter()
        .filter(|t| t.session_id == session_id)
        .max_by_key(|t| t.mtime)
        .map(|t| t.path)
}

/// 列出会话的子代理转录（M-C3）：`<session-id>/subagents/agent-<id>.jsonl`。
///
/// 新版 Claude Code 的子代理（Task/Agent 工具）消息写入独立文件而非主
/// transcript；这里只列出名字与任务描述（来自同目录 `agent-<id>.meta.json`
/// 的 description 字段），消息内容按需由前端点开再加载（本批只读列表）。
/// 目录不存在（无子代理的会话，绝大多数）返回空列表。
fn list_subagents(session_id: &str) -> Vec<SubagentInfo> {
    list_subagents_in(&projects_dir(), session_id)
}

/// [`list_subagents`] 的目录参数版（便于单测注入临时目录）。
fn list_subagents_in(projects_dir: &Path, session_id: &str) -> Vec<SubagentInfo> {
    scan_transcripts(projects_dir, true)
        .into_iter()
        // 子代理目录只会挂在它自己的会话目录下，按宿主会话 id 过滤即可
        .filter(|t| t.owner_session.as_deref() == Some(session_id))
        .map(|t| SubagentInfo {
            description: subagent_description(&t.path),
            id: t.session_id,
        })
        .collect()
}

/// 子代理任务描述：同目录 `agent-<id>.meta.json` 的 description（读不到为空串）。
fn subagent_description(transcript: &Path) -> String {
    std::fs::read_to_string(transcript.with_extension("meta.json"))
        .ok()
        .and_then(|c| serde_json::from_str::<serde_json::Value>(&c).ok())
        .and_then(|v| {
            v.get("description")
                .and_then(|x| x.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or_default()
}

/// 按优先级选出标题：用户自定义 > 会话 agentName > AI 总结 > 首条用户输入 > 占位符。
fn pick_title(
    custom_title: String,
    agent_name: String,
    ai_title: String,
    first_user_title: String,
) -> String {
    if !custom_title.is_empty() {
        custom_title
    } else if !agent_name.is_empty() {
        agent_name
    } else if !ai_title.is_empty() {
        ai_title
    } else if !first_user_title.is_empty() {
        first_user_title
    } else {
        UNNAMED_SESSION.to_string()
    }
}

/// 解析过程中的累加结果：概要采样（列表）与全量解析（详情）共用同一批字段。
///
/// 两个入口的差别只剩「读哪些行」和「要不要收集消息」，字段维护与兜底逻辑
/// 全在这里，避免同一条规则在两处各写一遍、改一处漏一处。
#[derive(Default)]
struct Draft {
    /// 用户自定义标题（`type:"custom-title"` 行的 customTitle）
    custom_title: String,
    /// 会话 agentName（`type:"agent-name"`）：没有 custom-title 时作标题
    agent_name: String,
    /// Claude Code 落盘的 AI 总结标题（`type:"summary"`）
    ai_title: String,
    /// 首条用户输入（标题的最后一级回退）
    first_user: String,
    cwd: String,
    project: String,
    last_activity: String,
    preview: String,
    messages: Vec<Message>,
    /// `tool_use_id → 工具名`：tool_result 配对用（M-C4，详见 [`parse_user_line`]）
    tool_names: HashMap<String, String>,
}

impl Draft {
    /// 记录工作目录：首次出现的 `cwd` 决定会话项目（后续行不再覆盖）。
    fn observe_cwd(&mut self, v: &serde_json::Value) {
        if self.cwd.is_empty() {
            if let Some(c) = v.get("cwd").and_then(|x| x.as_str()) {
                self.cwd = c.to_string();
                self.project = project_name_from_cwd(c);
            }
        }
    }

    /// 全量解析的每行公共字段：时间戳（后出现的覆盖前面的，最后一条即最后活动时间）
    /// 与工作目录。
    fn observe(&mut self, v: &serde_json::Value) {
        if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()) {
            self.last_activity = to_local_iso(ts);
        }
        self.observe_cwd(v);
    }

    /// 头窗口的一行：收集标题来源；返回「标题与 cwd 都齐了」（可提前收工）。
    fn observe_title_line(&mut self, kind: &str, v: &serde_json::Value) -> bool {
        match kind {
            "custom-title" => {
                if let Some(ct) = v.get("customTitle").and_then(|x| x.as_str()) {
                    self.custom_title = ct.to_string();
                }
            }
            "agent-name" => {
                if let Some(an) = v.get("agentName").and_then(|x| x.as_str()) {
                    self.agent_name = an.to_string();
                }
            }
            "summary" => {
                if let Some(s) = v.get("summary").and_then(|x| x.as_str()) {
                    self.ai_title = s.to_string();
                }
            }
            // 其余类型行与概要标题无关，跳过
            _ => {}
        }
        !self.custom_title.is_empty() && !self.agent_name.is_empty() && !self.cwd.is_empty()
    }

    /// 首条用户输入作为标题回退（惰性模式：只要标题与预览，不收集消息）。
    fn first_user_from_head(&mut self, head: &[String]) {
        let mut discard: Vec<Message> = Vec::new();
        for line in head {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            if v.get("type").and_then(|x| x.as_str()) != Some("user") {
                continue;
            }
            // 预览用临时变量：别覆盖尾部窗口即将提取的最新预览
            let mut head_preview = String::new();
            parse_user_line(
                &v,
                "",
                &mut self.first_user,
                &mut head_preview,
                &mut discard,
                true,
                &HashMap::new(),
            );
            if !self.first_user.is_empty() {
                break;
            }
        }
    }

    /// 尾窗口的一行：补最后活动时间与预览（不回填 cwd——那是头窗口的职责）。
    fn observe_tail(&mut self, v: &serde_json::Value) {
        if self.last_activity.is_empty() {
            if let Some(ts) = v.get("timestamp").and_then(|x| x.as_str()) {
                self.last_activity = to_local_iso(ts);
            }
        }
        if !self.preview.is_empty() {
            return;
        }
        let mut discard: Vec<Message> = Vec::new();
        match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
            "user" => {
                let mut title_tmp = String::new();
                parse_user_line(&v, "", &mut title_tmp, &mut self.preview, &mut discard, true, &HashMap::new());
            }
            "assistant" => {
                let mut tools: HashMap<String, String> = HashMap::new();
                parse_assistant_line(&v, "", &mut self.preview, &mut discard, true, &mut tools);
            }
            _ => {}
        }
    }

    /// 尾部没有可展示内容时，从头部窗口倒序找一条有内容的行（短会话场景）。
    fn preview_from_head(&mut self, head: &[String]) {
        for line in head.iter().rev() {
            let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
                continue;
            };
            // 全部用临时变量：这里只提取预览文本，不收集消息、不写标题
            let mut discard: Vec<Message> = Vec::new();
            let mut preview_tmp = String::new();
            match v.get("type").and_then(|x| x.as_str()).unwrap_or("") {
                "user" => {
                    let mut title_tmp = String::new();
                    parse_user_line(&v, "", &mut title_tmp, &mut preview_tmp, &mut discard, true, &HashMap::new());
                }
                "assistant" => {
                    let mut tools: HashMap<String, String> = HashMap::new();
                    parse_assistant_line(&v, "", &mut preview_tmp, &mut discard, true, &mut tools);
                }
                _ => continue,
            }
            if !preview_tmp.is_empty() {
                self.preview = preview_tmp;
                break;
            }
        }
    }

    /// 解析一行 user / assistant，写回时间戳、标题回退、预览与消息。
    fn observe_message_line(&mut self, kind: &str, v: &serde_json::Value, lazy: bool) {
        match kind {
            "user" => parse_user_line(
                v,
                &self.last_activity,
                &mut self.first_user,
                &mut self.preview,
                &mut self.messages,
                lazy,
                &self.tool_names,
            ),
            "assistant" => parse_assistant_line(
                v,
                &self.last_activity,
                &mut self.preview,
                &mut self.messages,
                lazy,
                &mut self.tool_names,
            ),
            _ => {}
        }
    }

    /// system 行按 subtype 区分（L-C11）：只展示有语义的行。
    fn observe_system_line(&mut self, v: &serde_json::Value, lazy: bool) {
        // 上下文压缩边界：对话被 compact，展示为提示行；未知 subtype 保守展示
        // （避免误丢新版本的有语义行），明确无语义的子类型跳过不入流。
        let subtype = v.get("subtype").and_then(|x| x.as_str()).unwrap_or("");
        if matches!(subtype, "turn_duration" | "stop_hook_summary" | "away_summary") {
            return;
        }
        let Some(text) = v.get("content").and_then(content_to_text) else {
            return;
        };
        if lazy {
            return;
        }
        let content = if subtype == "compact_boundary" {
            format!("上下文压缩边界：{text}")
        } else {
            text
        };
        self.messages
            .push(raw_message("system", "system", &content, &self.last_activity, None));
    }

    /// 标题优先级：用户自定义 > 会话 agentName > AI 总结 > 首条用户输入。
    fn title(&self) -> String {
        pick_title(
            self.custom_title.clone(),
            self.agent_name.clone(),
            self.ai_title.clone(),
            self.first_user.clone(),
        )
    }

    /// 汇总成 [`SessionInfo`]（列表与详情共用的出口）。
    ///
    /// 原生 transcript 不记录实时状态，历史会话一律 `completed`。
    fn finish(self, session_id: String) -> SessionInfo {
        // 先算标题：标题要从多个字段里挑，算完才能把字段 move 进结构体
        let title = self.title();
        SessionInfo {
            id: session_id,
            project: if self.project.is_empty() {
                UNKNOWN_PROJECT.to_string()
            } else {
                self.project
            },
            cwd: self.cwd,
            title,
            status: "completed".to_string(),
            last_activity: self.last_activity,
            preview: self.preview,
            unread: false,
            messages: self.messages,
            transcript_path: None,
            source: None,
            subagents: Vec::new(),
        }
    }
}

/// 概要模式解析：只读文件头尾采样（标题在头部，最新预览/时间在尾部），
/// 不收集消息，大文件不必全量读入。
///
/// - 头部窗口：cwd / 项目名 / custom-title / agent-name / summary / 首条用户输入（标题）；
/// - 尾部窗口：最新的预览与最后活动时间；
/// - 文件不超过窗口大小时等价于全量读取，行为与逐行全量解析一致。
fn parse_summary_head_tail(path: &Path) -> SessionInfo {
    let session_id = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    let head = read_head_lines_growing(path);
    let tail = read_tail_lines(path, SUMMARY_TAIL_BYTES);

    let mut d = Draft::default();

    // 头部窗口：cwd 与标题来源
    for line in &head {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // 非 JSON 行（脏数据）：跳过，不中断扫描
        };
        let Some(kind) = v.get("type").and_then(|x| x.as_str()) else {
            continue;
        };
        // 头部只认 cwd：最后活动时间一律以尾部窗口为准（否则短文件里
        // 头部的旧时间戳会被当成最新活动时间）
        d.observe_cwd(&v);
        if d.observe_title_line(kind, &v) {
            break; // 标题与 cwd 都拿到了，头部不必再扫
        }
    }
    d.first_user_from_head(&head);

    // 尾部窗口：倒序找最近一条带时间戳 / 内容的行
    for line in tail.iter().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        d.observe_tail(&v);
        if !d.last_activity.is_empty() && !d.preview.is_empty() {
            break;
        }
    }
    if d.preview.is_empty() {
        d.preview_from_head(&head);
    }

    d.finish(session_id)
}

/// 读取文件头部窗口内的完整行（最多 `window` 字节；首行可能被截断则丢弃）。
fn read_head_lines(path: &Path, window: u64) -> Vec<String> {
    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = f.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    if len <= window {
        let mut buf = String::new();
        f.read_to_string(&mut buf).ok();
        return collect_lines(&buf);
    }
    use std::io::Read;
    let mut buf = Vec::new();
    f.take(window).read_to_end(&mut buf).ok();
    // 尾行可能从多字节字符中间被截断，用 lossy 转换并丢弃尾行
    let text = String::from_utf8_lossy(&buf);
    let mut lines = collect_lines(&text);
    lines.pop();
    lines
}

/// L-C7：头窗口读取 + 自适应倍增。16KB 窗口可能被超长首行整个占掉
///（首行不完整即被丢弃，窗口内无任何完整行），或窗口内完整行都不带
/// cwd（超长首行挤掉了后续行）。此时逐级倍增窗口重读：16KB → 64KB →
/// 256KB 封顶，直到解析出至少一条含 cwd 的完整 JSON 行为止；
/// 窗口 >= 文件大小时等价于全量读取。
fn read_head_lines_growing(path: &Path) -> Vec<String> {
    let mut window = SUMMARY_HEAD_BYTES;
    loop {
        let lines = read_head_lines(path, window);
        // 已有含 cwd 的完整行：足够提取标题/项目名，直接用
        if lines.iter().any(|l| l.contains("\"cwd\"")) {
            return lines;
        }
        // 窗口内无完整行 / 完整行都不带 cwd：文件大小以内且未到上限则倍增重读
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        if window >= SUMMARY_HEAD_MAX_BYTES || window >= len {
            return lines;
        }
        window = (window * 4).min(SUMMARY_HEAD_MAX_BYTES);
    }
}

/// 读取文件尾部窗口内的完整行（最多 `window` 字节；首行可能被截断则丢弃）。
fn read_tail_lines(path: &Path, window: u64) -> Vec<String> {
    use std::io::{Read, Seek, SeekFrom};

    let Ok(mut f) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let Ok(meta) = f.metadata() else {
        return Vec::new();
    };
    let len = meta.len();
    if len <= window {
        let mut buf = String::new();
        f.read_to_string(&mut buf).ok();
        return collect_lines(&buf);
    }
    f.seek(SeekFrom::Start(len - window)).ok();
    let mut buf = Vec::new();
    f.take(window).read_to_end(&mut buf).ok();
    // 首行可能从多字节字符中间被截断，用 lossy 转换并丢弃首行
    let text = String::from_utf8_lossy(&buf);
    let mut lines = collect_lines(&text);
    if !lines.is_empty() {
        lines.remove(0);
    }
    lines
}

/// 文本按行收集（trim + 去空行）。
fn collect_lines(text: &str) -> Vec<String> {
    // 显式循环收集非空行（比 iter().map().filter().collect() 直白）
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.is_empty() {
            lines.push(trimmed.to_string());
        }
    }
    lines
}

/// 按需加载历史会话详情（原生 transcript，全量消息，不占常驻内存）。
///
/// M-C3：附带该会话 `subagents/` 目录下的子代理转录列表（只读列表，
/// 子代理消息在独立文件里，主 transcript 中只有 isSidechain 标记）。
pub fn load_session_detail(session_id: &str) -> Option<SessionInfo> {
    let path = native_session_path(session_id)?;
    let mut info = parse_native_session(&path, false)?;
    info.subagents = list_subagents(session_id);
    Some(info)
}

/// 解析单个原生会话文件（Claude Code 的 transcript .jsonl）。
///
/// `lazy = true`：概要模式（列表用），只提取标题/项目/时间/预览，不收集消息；
/// `false`：解析全部消息（用户点开会话详情时）。
///
/// L-C10：详情解析逐行流式读取（`BufReader::lines`），不再 `read_to_string`
/// 整个文件——逐行读仍会整体进 Vec，但避免了「超大单行 + 完整文件字符串」
/// 的双份驻留，超大首行（如超长 tool_result）不再复制两遍。
fn parse_native_session(path: &Path, lazy: bool) -> Option<SessionInfo> {
    let session_id = path.file_stem()?.to_string_lossy().to_string();
    let mut d = Draft::default();

    // 文件打不开即没有会话可解析（调用方返回 None），而不是给一个空会话
    if !for_each_json_line(path, |v| {
        let Some(t) = v.get("type").and_then(|x| x.as_str()) else {
            return;
        };
        d.observe(v);
        match t {
            // 标题三类来源（custom-title / agent-name / summary）的字段提取
            // 与概要模式共用
            "custom-title" | "agent-name" | "summary" => {
                d.observe_title_line(t, v);
            }
            // M-C3：新版子代理消息写入独立文件（<session-id>/subagents/
            // agent-<id>.jsonl），主 transcript 里不再混入子代理内容；
            // 旧版本仍可能把 isSidechain=true 的行混在主文件里，兼容跳过
            "user" | "assistant" if is_sidechain(v) => {}
            "user" | "assistant" => d.observe_message_line(t, v, lazy),
            // L-C11：system 行按 subtype 区分——只展示有语义的行
            "system" => d.observe_system_line(v, lazy),
            // 其余未知类型（hook 行、file-history-snapshot 等）：与展示无关，跳过
            _ => {}
        }
    }) {
        return None;
    }

    Some(d.finish(session_id))
}

/// 该行是否为子代理（Task/Agent 工具）产生的内容。
fn is_sidechain(v: &serde_json::Value) -> bool {
    v.get("isSidechain").and_then(|x| x.as_bool()).unwrap_or(false)
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


/// 行解析的输出侧：预览文本、消息列表、惰性开关与行时间戳。
///
/// 文本 / 工具结果 / 图片三类块都要往这几样里写，打包成一个结构体传递：
/// 处理函数不必各拖一串 `&mut` 参数，`lazy` 的取舍也只写在这一处。
struct LineOut<'a> {
    /// 该行的时间戳（已转本地时间），写入它产生的每条消息
    time: &'a str,
    /// 概要模式：只取预览，不收集消息（列表页不需要全量消息）
    lazy: bool,
    preview: &'a mut String,
    messages: &'a mut Vec<Message>,
}

impl LineOut<'_> {
    /// 更新预览（列表页靠它显示摘要）。
    fn preview(&mut self, text: &str) {
        *self.preview = truncate(text, 80);
    }

    /// 收集一条消息（惰性模式下丢弃）。
    fn push(
        &mut self,
        msg_type: &'static str,
        role: &'static str,
        content: &str,
        tool_call: Option<String>,
    ) {
        if self.lazy {
            return;
        }
        self.messages
            .push(raw_message(msg_type, role, content, self.time, tool_call));
    }

    /// 预览 + 收集（最常见的组合）。
    fn show(
        &mut self,
        msg_type: &'static str,
        role: &'static str,
        content: &str,
        tool_call: Option<String>,
    ) {
        self.preview(content);
        self.push(msg_type, role, content, tool_call);
    }
}

/// 解析一个 `tool_result` 块，返回「展示文本 + 所属工具名」。
///
/// user 行与 assistant 行都可能出现 tool_result（新老数据格式差异），
/// 配对（M-C4：`tool_use_id` → `tool_names`）与失败标记的规则完全一致，
/// 差别只在 user 行还要顺带更新预览——那一步留给调用方。
///
/// - M-C4：`is_error: true` 的结果加失败标记前缀（前端按前缀渲染失败样式）；
/// - M-C5：content 里的 image 块转「[图片 N]」占位文本（不透传 base64 数据）。
fn tool_result_block(
    block: &serde_json::Value,
    tool_names: &HashMap<String, String>,
    img_no: &mut usize,
) -> Option<(String, Option<String>)> {
    let tool_name = block
        .get("tool_use_id")
        .and_then(|x| x.as_str())
        .and_then(|id| tool_names.get(id))
        .cloned();
    let mut text = content_to_text_with_images(block.get("content")?, img_no)?;
    if block
        .get("is_error")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        text = format!("[工具执行失败]\n{text}");
    }
    Some((text, tool_name))
}

/// 解析原生 transcript 中的 `user` 行：
/// - 字符串 content 为真实用户输入；系统注入的命令回显/compact 摘要归为 system；
/// - 块数组 content 中的 `tool_result` 为工具执行结果、`text` 为用户文本。
///
/// 注入识别：结构化字段（isCompactSummary / isMeta / origin.kind / promptSource）
/// 优先，无结构化信号（旧版本数据）回退到 `<` 前缀标记。
///
/// - M-C4：tool_result 带 `tool_use_id`，用 `tool_names`（此前的 tool_use
///   块构建的 id → 工具名映射）把它配对回所属工具，`is_error: true` 的结果
///   加失败标记前缀（前端按前缀渲染失败样式）。
/// - M-C5：image 块生成「[图片 N]」占位文本（不透传 base64 数据）。
fn parse_user_line(
    v: &serde_json::Value,
    time: &str,
    title: &mut String,
    preview: &mut String,
    messages: &mut Vec<Message>,
    lazy: bool,
    tool_names: &HashMap<String, String>,
) {
    let Some(msg) = v.get("message") else { return };
    let Some(content) = msg.get("content") else { return };
    let mut out = LineOut {
        time,
        lazy,
        preview,
        messages,
    };

    match content {
        serde_json::Value::String(s) => user_text_block(v, s, title, &mut out),
        serde_json::Value::Array(blocks) => {
            // M-C5：块内图片计数（每个 image 块一个占位，编号在一次消息内连续）
            let mut img_no = 0usize;
            for block in blocks {
                user_content_block(block, title, tool_names, &mut img_no, &mut out);
            }
        }
        _ => {}
    }
}

/// `user` 行的字符串 content（真实用户输入 / 命令回显 / compact 摘要）。
fn user_text_block(v: &serde_json::Value, s: &str, title: &mut String, out: &mut LineOut) {
    // 结构化信号指向注入（compact 摘要 / isMeta / tool 来源等）直接归 system；
    // 其余（human 或无信号）仍走 < 前缀判断——覆盖命令回显
    // （origin.kind=human 但内容以 <command- 开头）与旧版本数据
    let is_marker = injected_user_line(v) == Some(true) || is_system_marker(s);
    if title.is_empty() && !is_marker {
        *title = truncate(s, 40);
    }
    let kind = if is_marker { "system" } else { "user" };
    out.show(kind, kind, s, None);
}

/// `user` 行内容数组里的一个块。
fn user_content_block(
    block: &serde_json::Value,
    title: &mut String,
    tool_names: &HashMap<String, String>,
    img_no: &mut usize,
    out: &mut LineOut,
) {
    match block.get("type").and_then(|x| x.as_str()) {
        // M-C4：工具执行结果（预览与消息都要，配对逻辑与 assistant 行共用）
        Some("tool_result") => {
            if let Some((text, tool_name)) = tool_result_block(block, tool_names, img_no) {
                out.show("tool_result", "assistant", &text, tool_name);
            }
        }
        // M-C5：user 消息块的 image → 「[图片 N]」占位
        Some("image") => {
            *img_no += 1;
            let text = format!("[图片 {}]", img_no);
            out.show("user", "user", &text, None);
        }
        Some("text") => {
            if let Some(text) = block_text(block) {
                if title.is_empty() {
                    *title = truncate(&text, 40);
                }
                out.show("user", "user", &text, None);
            }
        }
        // 未知 block 类型：跳过（不做处理，也不报错）
        _ => {}
    }
}

/// 解析原生 transcript 中的 `assistant` 行，忠实保留每个内容块：
/// - `text` 块 → assistant 文本；
/// - `thinking` 块 → 思考过程（thinking 类型）；
/// - `tool_use` 块 → 工具调用（带 toolCall 徽标与入参），M-C4：块 id
///   登记进 `tool_names`（后续 tool_result 按 tool_use_id 配对回工具名）；
/// - `tool_result` 块 → 工具结果（老格式数据，同 user 行的 tool_result 处理）。
fn parse_assistant_line(
    v: &serde_json::Value,
    time: &str,
    preview: &mut String,
    messages: &mut Vec<Message>,
    lazy: bool,
    tool_names: &mut HashMap<String, String>,
) {
    let Some(msg) = v.get("message") else { return };
    let Some(content) = msg.get("content") else { return };
    let mut out = LineOut {
        time,
        lazy,
        preview,
        messages,
    };

    match content {
        serde_json::Value::String(s) => out.show("assistant", "assistant", s, None),
        serde_json::Value::Array(blocks) => {
            let mut img_no = 0usize;
            for block in blocks {
                assistant_content_block(block, tool_names, &mut img_no, &mut out);
            }
        }
        _ => {}
    }
}

/// `assistant` 行内容数组里的一个块。
fn assistant_content_block(
    block: &serde_json::Value,
    tool_names: &mut HashMap<String, String>,
    img_no: &mut usize,
    out: &mut LineOut,
) {
    match block.get("type").and_then(|x| x.as_str()) {
        Some("text") => {
            if let Some(t) = block.get("text").and_then(|x| x.as_str()) {
                out.show("assistant", "assistant", t, None);
            }
        }
        // 思考过程：不进预览（预览展示「说了什么/做了什么」，不展示内心戏）
        Some("thinking") => {
            if let Some(t) = block.get("thinking").and_then(|x| x.as_str()) {
                out.push("thinking", "assistant", t, None);
            }
        }
        Some("tool_use") => {
            // M-C4：登记 id → 工具名，供后续 tool_result 配对
            let name = block
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("工具")
                .to_string();
            if let Some(id) = block.get("id").and_then(|x| x.as_str()) {
                tool_names.insert(id.to_string(), name.clone());
            }
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
            out.push("tool_use", "assistant", &content, Some(name));
        }
        // 老格式数据：工具结果落在 assistant 行里（不进预览）
        Some("tool_result") => {
            if let Some((text, tool_name)) = tool_result_block(block, tool_names, img_no) {
                out.push("tool_result", "assistant", &text, tool_name);
            }
        }
        // 未知 block 类型：跳过（不做处理，也不报错）
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
///
/// M-C5：块数组里的 image 块转「[图片 N]」占位文本（N 从 1 递增，
/// `img_no` 由调用方传入保证一次消息里多图的编号连续），不透传 base64 数据。
fn content_to_text_with_images(content: &serde_json::Value, img_no: &mut usize) -> Option<String> {
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
                        // M-C5：图片块 → 占位文本（不做缩略图，进阶留待迭代）
                        Some("image") => {
                            *img_no += 1;
                            parts.push(format!("[图片 {}]", *img_no));
                        }
                        Some("tool_result") => {
                            if let Some(c) = o.get("content") {
                                if let Some(t) = content_to_text_with_images(c, img_no) {
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

/// 从 content 值提取纯文本（不带图片计数，语义同 [`content_to_text_with_images`] 的 `img_no=0`）。
fn content_to_text(content: &serde_json::Value) -> Option<String> {
    content_to_text_with_images(content, &mut 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// 写一个最小 transcript 文件到临时目录。
    fn write_transcript(dir: &Path, session_id: &str, lines: &[String]) -> PathBuf {
        let file = dir.join(format!("{session_id}.jsonl"));
        let mut f = std::fs::File::create(&file).unwrap();
        for l in lines {
            writeln!(f, "{l}").unwrap();
        }
        file
    }

    fn user_line(prompt: &str, ts: &str) -> String {
        format!(r#"{{"type":"user","cwd":"D:/work/proj","timestamp":"{ts}","message":{{"content":"{prompt}"}}}}"#)
    }

    fn assistant_line(text: &str, ts: &str) -> String {
        format!(r#"{{"type":"assistant","timestamp":"{ts}","message":{{"content":[{{"type":"text","text":"{text}"}}]}}}}"#)
    }

    #[test]
    fn summary_head_tail_extracts_title_and_preview() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-summary-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 头部：首条用户输入（标题来源）；尾部：最新 assistant 文本（预览来源）
        let lines = vec![
            user_line("帮我修复登录问题", "2026-08-20T10:00:00Z"),
            assistant_line("正在分析", "2026-08-20T10:01:00Z"),
            assistant_line("最终结论：问题已修复", "2026-08-20T10:05:00Z"),
        ];
        let path = write_transcript(&dir, "sess-1", &lines);

        let info = parse_summary_head_tail(&path);
        assert_eq!(info.id, "sess-1");
        assert_eq!(info.title, "帮我修复登录问题");
        assert_eq!(info.project, "proj");
        // 尾部采样应取到最新一条 assistant 文本作为预览
        assert!(info.preview.contains("最终结论"), "preview={}", info.preview);
        assert!(!info.last_activity.is_empty());
        assert!(info.messages.is_empty(), "概要不收集消息");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_prefers_custom_title() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-title-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            r#"{"type":"user","cwd":"D:/work/proj","timestamp":"2026-08-20T10:00:00Z","message":{"content":"很长的首条用户输入内容作为标题回退"}}"#.to_string(),
            r#"{"type":"custom-title","customTitle":"自定义标题"}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-2", &lines);

        let info = parse_summary_head_tail(&path);
        assert_eq!(info.title, "自定义标题");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summary_prefers_agent_name_over_ai_title() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-agent-name-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            r#"{"type":"user","cwd":"D:/work/proj","timestamp":"2026-08-20T10:00:00Z","message":{"content":"很长的首条用户输入内容作为标题回退"}}"#.to_string(),
            r#"{"type":"summary","summary":"AI 总结标题"}"#.to_string(),
            r#"{"type":"agent-name","agentName":"会话名"}"#.to_string(),
            r#"{"type":"custom-title","customTitle":"自定义标题"}"#.to_string(),
        ];

        // custom-title 缺失：agentName 优先于 AI 总结与首条用户输入
        let no_custom = write_transcript(&dir, "sess-3", &lines[..3]);
        assert_eq!(parse_summary_head_tail(&no_custom).title, "会话名");

        // custom-title 存在：优先级最高
        let with_custom = write_transcript(&dir, "sess-4", &lines);
        assert_eq!(parse_summary_head_tail(&with_custom).title, "自定义标题");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detail_parses_full_messages() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-detail-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            user_line("第一个问题", "2026-08-20T10:00:00Z"),
            assistant_line("第一个回答", "2026-08-20T10:01:00Z"),
        ];
        let path = write_transcript(&dir, "sess-3", &lines);

        let info = parse_native_session(&path, false).unwrap();
        assert_eq!(info.messages.len(), 2);
        assert_eq!(info.messages[0].msg_type, "user");
        assert_eq!(info.messages[0].content, "第一个问题");
        assert_eq!(info.messages[1].msg_type, "assistant");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn compact_summary_is_system_not_user() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-compact-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // compact_boundary 后的续接摘要：带 isCompactSummary 结构化标记，
        // 不以 < 开头，旧逻辑会误判为真实用户输入
        let lines = vec![
            user_line("真实问题", "2026-08-20T10:00:00Z"),
            r#"{"type":"user","isCompactSummary":true,"isVisibleInTranscriptOnly":true,"timestamp":"2026-08-20T10:10:00Z","message":{"content":"This session is being continued from a previous conversation..."}}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-4", &lines);

        let info = parse_native_session(&path, false).unwrap();
        // 摘要归为 system，不出现假的用户消息
        assert_eq!(info.messages.len(), 2);
        assert_eq!(info.messages[0].msg_type, "user");
        assert_eq!(info.messages[1].msg_type, "system");
        // 标题仍来自首条真实输入，而非 compact 摘要
        assert_eq!(info.title, "真实问题");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn origin_kind_overrides_marker_prefix() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-origin-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // human 输入以 < 开头（如 /init 命令回显 origin.kind=human）：结构化字段
        // 优先于前缀匹配时仍归为注入（本地命令回显）；tool 来源同样归为注入
        let lines = vec![
            r#"{"type":"user","origin":{"kind":"tool"},"timestamp":"2026-08-20T10:00:00Z","message":{"content":"tool 产生的字符串输入"}}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-5", &lines);

        let info = parse_native_session(&path, false).unwrap();
        assert_eq!(info.messages.len(), 1);
        assert_eq!(info.messages[0].msg_type, "system", "origin.kind=tool 应归为 system");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn sidechain_lines_skipped_in_detail() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-sidechain-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 子代理消息（isSidechain=true）混在同一 transcript 文件，不应进入主流
        let lines = vec![
            user_line("主流程问题", "2026-08-20T10:00:00Z"),
            r#"{"type":"assistant","isSidechain":true,"timestamp":"2026-08-20T10:01:00Z","message":{"content":[{"type":"text","text":"子代理内部输出"}]}}"#.to_string(),
            r#"{"type":"user","isSidechain":true,"timestamp":"2026-08-20T10:02:00Z","message":{"content":"子代理内部用户行"}}"#.to_string(),
            assistant_line("主流程回答", "2026-08-20T10:03:00Z"),
        ];
        let path = write_transcript(&dir, "sess-6", &lines);

        let info = parse_native_session(&path, false).unwrap();
        assert_eq!(info.messages.len(), 2, "sidechain 行应被跳过");
        assert_eq!(info.messages[0].content, "主流程问题");
        assert_eq!(info.messages[1].content, "主流程回答");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-C4：tool_use 的 id 与 tool_result 的 tool_use_id 配对——
    /// result 的 toolCall 带上所属工具名；is_error=true 加失败标记。
    #[test]
    fn tool_result_paired_to_tool_use() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-pairing-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            // assistant 行带 tool_use（id=call-1，工具名 Bash）
            r#"{"type":"assistant","timestamp":"2026-08-20T10:00:00Z","message":{"content":[{"type":"tool_use","id":"call-1","name":"Bash","input":{"command":"ls"}}]}}"#.to_string(),
            // user 行带 tool_result（tool_use_id=call-1，正常结果）
            r#"{"type":"user","timestamp":"2026-08-20T10:00:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-1","content":"file-a\nfile-b"}]}}"#.to_string(),
            // 第二个 tool_use + 失败的 tool_result（is_error=true）
            r#"{"type":"assistant","timestamp":"2026-08-20T10:01:00Z","message":{"content":[{"type":"tool_use","id":"call-2","name":"Read","input":{"file_path":"x.txt"}}]}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-08-20T10:01:05Z","message":{"content":[{"type":"tool_result","tool_use_id":"call-2","is_error":true,"content":"文件不存在"}]}}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-pair", &lines);

        let info = parse_native_session(&path, false).unwrap();
        let tool_results: Vec<&Message> = info
            .messages
            .iter()
            .filter(|m| m.msg_type == "tool_result")
            .collect();
        assert_eq!(tool_results.len(), 2);

        // 正常结果：配对到工具名 Bash，无失败标记
        assert_eq!(tool_results[0].tool_call.as_deref(), Some("Bash"), "tool_result 应配对到所属工具名");
        assert_eq!(tool_results[0].content, "file-a\nfile-b");

        // 失败结果：配对到 Read，内容带失败标记前缀
        assert_eq!(tool_results[1].tool_call.as_deref(), Some("Read"));
        assert!(tool_results[1].content.starts_with("[工具执行失败]"), "is_error 结果应带失败标记: {}", tool_results[1].content);
        assert!(tool_results[1].content.contains("文件不存在"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-C5：user 消息与 tool_result 里的 image 块生成「[图片 N]」占位，
    /// base64 数据不透传。
    #[test]
    fn image_blocks_become_placeholders() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-image-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            // user 行：块数组里 image + text 混合
            r#"{"type":"user","cwd":"D:/work/proj","timestamp":"2026-08-20T10:00:00Z","message":{"content":[{"type":"image","source":{"type":"base64","media_type":"image/png","data":"AAAABBBBCCCC"}},{"type":"text","text":"看这张图"}]}}"#.to_string(),
            // tool_result 的 content 数组里嵌 image
            r#"{"type":"assistant","timestamp":"2026-08-20T10:00:10Z","message":{"content":[{"type":"tool_use","id":"c1","name":"Screenshot","input":{}}]}}"#.to_string(),
            r#"{"type":"user","timestamp":"2026-08-20T10:00:15Z","message":{"content":[{"type":"tool_result","tool_use_id":"c1","content":[{"type":"text","text":"截图如下"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":"DDDD"}}]}]}}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-img", &lines);

        let info = parse_native_session(&path, false).unwrap();
        let all: Vec<&Message> = info.messages.iter().collect();
        // user 消息两块 → 两条消息（[图片 1] + 文本）
        assert!(all.iter().any(|m| m.msg_type == "user" && m.content == "[图片 1]"), "user 的 image 块应生成占位");
        assert!(all.iter().any(|m| m.msg_type == "user" && m.content == "看这张图"));
        // tool_result：文本 + 占位合并成一条（编号按行内顺序递增）
        let tr = all.iter().find(|m| m.msg_type == "tool_result").unwrap();
        assert!(tr.content.contains("[图片 1]"), "tool_result 内嵌 image 应生成占位: {}", tr.content);
        assert!(tr.content.contains("截图如下"));
        // base64 数据不应出现在任何消息里
        assert!(!all.iter().any(|m| m.content.contains("AAAABBBBCCCC")));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-C11：system 行按 subtype 区分——compact_boundary 展示为
    /// 「上下文压缩边界」，turn_duration / stop_hook_summary 跳过。
    #[test]
    fn system_lines_filtered_by_subtype() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-system-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            user_line("正常问题", "2026-08-20T10:00:00Z"),
            r#"{"type":"system","subtype":"compact_boundary","content":"Conversation compacted","timestamp":"2026-08-20T10:01:00Z"}"#.to_string(),
            r#"{"type":"system","subtype":"turn_duration","durationMs":188069,"content":"耗时","timestamp":"2026-08-20T10:02:00Z"}"#.to_string(),
            r#"{"type":"system","subtype":"stop_hook_summary","hookCount":2,"content":"hook 统计","timestamp":"2026-08-20T10:03:00Z"}"#.to_string(),
            r#"{"type":"system","subtype":"away_summary","content":"离开摘要","timestamp":"2026-08-20T10:04:00Z"}"#.to_string(),
        ];
        let path = write_transcript(&dir, "sess-sys", &lines);

        let info = parse_native_session(&path, false).unwrap();
        let systems: Vec<&Message> = info
            .messages
            .iter()
            .filter(|m| m.msg_type == "system")
            .collect();
        // 只有 compact_boundary 入流，其余三个 subtype 被过滤
        assert_eq!(systems.len(), 1, "turn_duration/stop_hook_summary/away_summary 应被跳过");
        assert!(systems[0].content.contains("上下文压缩边界"), "compact_boundary 应展示为语义化提示: {}", systems[0].content);
        assert!(systems[0].content.contains("Conversation compacted"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-C6：同 id 多文件按 mtime 最新去重（纯函数验证）。
    #[test]
    fn dedup_latest_by_id_keeps_newest() {
        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + std::time::Duration::from_secs(100);
        let files = vec![
            (PathBuf::from("projA/sess.jsonl"), "sess".to_string(), t0),
            (PathBuf::from("projB/sess.jsonl"), "sess".to_string(), t1),
            (PathBuf::from("projA/other.jsonl"), "other".to_string(), t0),
        ];
        let out = dedup_latest_by_id(files);
        // sess 只留 mtime 最新的 projB，other 保留
        assert_eq!(out.len(), 2);
        assert_eq!(out[0], (PathBuf::from("projB/sess.jsonl"), "sess".to_string(), t1));
        assert_eq!(out[1], (PathBuf::from("projA/other.jsonl"), "other".to_string(), t0));
    }

    /// L-C7：首行超过 16KB（头窗口无完整行）时倍增窗口重读，
    /// 概要仍能提取标题。
    #[test]
    fn summary_head_window_grows_for_long_first_line() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-longline-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // 首行：超长 user 输入（> 16KB，把默认头窗口整个占掉）
        let long_prompt = "超长首条用户输入内容标题".to_string() + &"x".repeat(20 * 1024);
        let mut lines = vec![user_line(&long_prompt, "2026-08-20T10:00:00Z")];
        lines.push(assistant_line("回答", "2026-08-20T10:05:00Z"));
        let path = write_transcript(&dir, "sess-long", &lines);

        let info = parse_summary_head_tail(&path);
        // 16KB 窗口读不到任何完整行时应倍增窗口，标题仍能提取（40 字符截断）
        assert!(info.title.starts_with("超长首条用户输入内容标题"), "title={}", info.title);
        assert_eq!(info.project, "proj");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-C3：`<session-id>/subagents/agent-<id>.jsonl` 被列为子代理会话，
    /// 描述取自 meta.json；无 subagents 目录的会话返回空列表。
    #[test]
    fn list_subagents_finds_agent_files() {
        let dir = std::env::temp_dir().join("ccbuddy-mgr-subagent-test");
        let _ = std::fs::remove_dir_all(&dir);
        let proj = dir.join("D--work-proj");
        let sub = proj.join("sess-main").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();

        // 子代理转录 + meta 描述
        std::fs::write(sub.join("agent-abc.jsonl"), "{}\n").unwrap();
        std::fs::write(
            sub.join("agent-abc.meta.json"),
            r#"{"agentType":"general-purpose","description":"修复登录问题","toolUseId":"call_1"}"#,
        )
        .unwrap();
        std::fs::write(sub.join("notes.txt"), "无关文件").unwrap();

        // list_subagents 扫描的是 projects 根目录下的 <session-id>/subagents/，
        // 测试对临时根目录验证同一逻辑
        let listed = list_subagents_in(&dir, "sess-main");
        assert_eq!(listed.len(), 1, "只列 .jsonl 转录（notes.txt 不算）");
        assert_eq!(listed[0].id, "agent-abc");
        assert_eq!(listed[0].description, "修复登录问题");

        // 无 subagents 目录：空列表
        let listed = list_subagents_in(&dir, "sess-none");
        assert!(listed.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }
}
