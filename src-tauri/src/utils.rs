//! 基础工具模块：前后端共享的数据模型与通用辅助函数。
//!
//! 被 [`crate::claude`]（原生 transcript 解析）与 [`crate::events_mgr`]
//! （事件流解析）共同使用，只放与具体数据源无关的基础性内容：
//! - `Message` / `SessionInfo`：RPC 返回给前端的展示模型
//! - 时间处理（`to_local_iso` / `short_time`）、文本截断、系统标记识别

use serde::Serialize;
use std::path::Path;

/// 会话拿不出任何标题时的占位（首条用户输入滚出窗口 / 只有系统事件）。
///
/// 事件流与原生 transcript 两条解析链都要用它：既是展示兜底，
/// 也是「标题还没解析出来，可以用旧值/快照垫底」的判定条件。
pub const UNNAMED_SESSION: &str = "(未命名会话)";

/// 拿不到项目名时的占位（transcript / 事件里没有可用的 cwd）。
pub const UNKNOWN_PROJECT: &str = "unknown";

/// 前端展示用的单条消息。
#[derive(Debug, Clone, Serialize)]
pub struct Message {
    /// "user" | "assistant" | "system" | "thinking" | "tool_use" | "tool_result"
    #[serde(rename = "type")]
    pub msg_type: &'static str,
    // 'static lifetime 是因为这些字符串是硬编码的常量，编译时就确定了它们的生命周期，因此可以安全地使用 'static。
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
    /// 消息列表。列表查询（懒加载）时为空，由详情命令按需填充。
    #[serde(default)]
    pub messages: Vec<Message>,
    /// 原生 transcript 路径（事件流会话携带；历史会话解析不提供）。
    #[serde(rename = "transcriptPath", skip_serializing_if = "Option::is_none")]
    pub transcript_path: Option<String>,
    /// SessionStart 来源（startup / resume / clear / compact / fork）。
    #[serde(rename = "source", skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// 子代理转录列表（M-C3：`<session-id>/subagents/agent-<id>.jsonl`），
    /// 历史会话详情返回；列表查询 / 事件流会话不提供。
    #[serde(rename = "subagents", skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubagentInfo>,
}

/// 子代理（Task/Agent 工具）转录的概要信息（M-C3）。
#[derive(Debug, Clone, Serialize)]
pub struct SubagentInfo {
    /// 子代理名（文件名派生，如 agent-a1444bf34ebff26b2）
    pub id: String,
    /// 任务描述（agent-<id>.meta.json 的 description；缺失为空串）
    pub description: String,
}

/// 用量记录：一次独立 API 请求的 token 计量（用量界面展示）。
///
/// 数据来源为原生 transcript 中 assistant 行的 `message.usage` 字段；
/// 全零 usage 的流式块影子行已在解析层过滤（见 [`crate::usage_mgr`]）。
#[derive(Debug, Clone, Serialize)]
pub struct UsageRecord {
    /// 请求时间（本地时区 ISO）
    pub timestamp: String,
    #[serde(rename = "sessionId")]
    pub session_id: String,
    /// 模型名，如 claude-sonnet-5（缺失时为空串）
    pub model: String,
    #[serde(rename = "inputTokens")]
    pub input_tokens: u64,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read_tokens: u64,
    #[serde(rename = "cacheCreationTokens")]
    pub cache_creation_tokens: u64,
    #[serde(rename = "outputTokens")]
    pub output_tokens: u64,
    /// 思考 token（部分模型/版本才有，缺失不序列化）
    #[serde(rename = "thinkingTokens", skip_serializing_if = "Option::is_none")]
    pub thinking_tokens: Option<u64>,
    /// 服务档位（部分版本才有，缺失不序列化）
    #[serde(rename = "serviceTier", skip_serializing_if = "Option::is_none")]
    pub service_tier: Option<String>,
    /// 本次请求成本（美元，新版 transcript 的 usage 带 prompt_cost /
    /// completion_cost / cache_cost；老版本无成本字段，缺失不序列化）
    #[serde(rename = "cost", skip_serializing_if = "Option::is_none")]
    pub cost: Option<f64>,
    /// 是否子代理（subagent）转录产生的用量；主会话恒为 false
    #[serde(rename = "isSubagent", default)]
    pub is_subagent: bool,
}

/// 截断字符串到指定字符数（按字符而非字节）。
pub fn truncate(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        s.to_string()
    } else {
        let t: String = s.chars().take(max_chars).collect();
        format!("{t}…")
    }
}

/// 逐行读一个 jsonl 文件，把每个能解析成 JSON 的行交给 `f`。
///
/// 事件日志、原生 transcript、用量统计三处的逐行读取规则完全一致，都在这里：
/// - 空行与非 JSON 行（脏数据）跳过，不打断整份文件；
/// - 读取中断（文件正在被追加写的竞态）用已读到的部分收尾；
/// - 逐行读而非 `read_to_string`（L-C10）：省去「超大单行 + 整个文件字符串」
///   的双份驻留。
///
/// 返回 false 表示文件打不开，调用方自行决定是返回空结果还是失败。
/// 闭包只拿到已解析的行，逐行状态（累加器、去重表）由调用方在自己的闭包里维护。
pub fn for_each_json_line(path: &Path, mut f: impl FnMut(&serde_json::Value)) -> bool {
    use std::io::BufRead;

    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    for line in std::io::BufReader::new(file).lines() {
        let Ok(line) = line else {
            break; // 读取中断：用已解析部分
        };
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else {
            continue; // 非 JSON 行：跳过
        };
        f(&v);
    }
    true
}

/// 读 Claude Code 的 `settings.json`：缺失 / 损坏一律返回 Null。
///
/// 只读方（保留期、hook 注册状态）都是「有就用、没有就算了」，
/// 因此错误统一吞掉；安装 / 卸载 hook 那条写路径需要把解析错误报给用户，
/// 自己读、不走这里。
pub fn read_settings_json(claude_dir: &Path) -> serde_json::Value {
    std::fs::read_to_string(claude_dir.join("settings.json"))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or(serde_json::Value::Null)
}

/// 从工作目录提取项目名（路径最后一段目录名），如 `D:/work/proj` → `proj`。
///
/// 事件流（hook 事件里的 cwd）与原生 transcript（行里的 cwd）共用同一规则：
/// 两处必须给出同一个项目名，否则同一项目的会话会被分到两个分组里。
/// 末尾斜杠先去掉，避免 `D:/work/proj/` 提取出空串。
pub fn project_name_from_cwd(cwd: &str) -> String {
    cwd.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(cwd)
        .to_string()
}

/// 判断 transcript `user` 行是否带结构化的"系统注入"信号。
///
/// 新版 Claude Code 提供可靠信号，避免了仅靠 `<` 前缀猜测：
/// - `isCompactSummary` / `isVisibleInTranscriptOnly` / `isMeta` 为 true → 注入；
/// - `origin.kind` 非 `human`（tool 等）→ 注入；
/// - `promptSource` 非 `typed` / `queued` → 注入。
///
/// 返回 `Some(true)` 表示确认注入；`Some(false)` / `None` 表示无注入信号，
/// 由调用方回退到 [`is_system_marker`] 的 `<` 前缀识别（覆盖命令回显
/// （`origin.kind=human` 但内容以 `<command-` 开头）与旧版本数据）。
pub fn injected_user_line(v: &serde_json::Value) -> Option<bool> {
    // 布尔型注入标记：任一为 true 即确认注入
    for key in ["isCompactSummary", "isVisibleInTranscriptOnly", "isMeta"] {
        if v.get(key).and_then(|x| x.as_bool()) == Some(true) {
            return Some(true);
        }
    }

    // origin.kind 存在且不是 human（如 tool 产生的输入）→ 注入；
    // 是 human 则继续检查后面的信号（不能直接返回 Some(false)，
    // 因为后续 promptSource 检查仍可能命中）
    let origin_kind: Option<String> = v
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    if let Some(kind) = origin_kind {
        if kind != "human" {
            return Some(true);
        }
    }

    // promptSource 非 typed/queued（如 slash 命令展开）→ 注入
    let prompt_source: Option<&str> = v.get("promptSource").and_then(|x| x.as_str());
    if let Some(ps) = prompt_source {
        if !matches!(ps, "typed" | "queued") {
            return Some(true);
        }
    }

    // 所有结构化信号都不存在：返回 None，让调用方回退到 < 前缀识别
    None
}

/// 判断文本是否是 Claude Code 系统注入标记（非真实用户输入）。
pub fn is_system_marker(s: &str) -> bool {
    let t = s.trim_start();
    // 不以 < 开头：一定不是系统标记，直接短路
    if !t.starts_with('<') {
        return false;
    }
    // 显式 for 循环逐个前缀比对（替代 iter().any() 链式写法）
    const MARKER_PREFIXES: [&str; 15] = [
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
    ];
    for prefix in MARKER_PREFIXES {
        if t.starts_with(prefix) {
            return true;
        }
    }
    false
}

/// 把 ISO 时间戳（UTC "Z" 或带偏移的本地时间）归一为本地时区时间。
///
/// hook 日志新格式为本地时间（带偏移），旧格式为 UTC；原生 transcript 为 UTC。
/// 统一转为本地时间后，字符串排序与完整时间展示（`short_time`）才正确。
/// 解析失败时原样返回（容忍脏数据）。
pub fn to_local_iso(iso: &str) -> String {
    match chrono::DateTime::parse_from_rfc3339(iso) {
        Ok(dt) => dt
            .with_timezone(&chrono::Local)
            .format("%Y-%m-%dT%H:%M:%S%.3f%:z")
            .to_string(),
        Err(_) => iso.to_string(),
    }
}

/// 把 ISO 时间戳转为完整本地时间 "YYYY-MM-DD HH:MM:SS"（消息行时间展示用）。
pub fn short_time(iso: &str) -> String {
    // 先归一为本地时区：hook 新日志带本地偏移，旧日志与原生 transcript 为 UTC，
    // to_local_iso 统一处理；再取日期段 + "T" 后前 8 位（HH:MM:SS）。
    let local = to_local_iso(iso);
    match local.split_once('T') {
        Some((date, t)) => {
            let hms: String = t.chars().take(8).collect();
            format!("{date} {hms}")
        }
        // 没有 'T'（非标准格式）：原样返回，容忍脏数据
        None => local,
    }
}
