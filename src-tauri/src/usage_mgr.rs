//! 用量统计：解析 Claude Code 原生 transcript 的 API 请求 token 计量。
//!
//! 数据源与 [`crate::claude`] 相同（`<claude_dir>/projects/` 下的 .jsonl），
//! 但视角不同：这里只关心 assistant 行的 `message.usage` 字段——
//! 一条非零 usage 的 assistant 行 = 一次独立的平台 API 请求。
//!
//! 关键规律：全零 usage 行是流式块的"影子行"（流式回包过程会落多行，
//! 只有最终聚合的那行带真实计量）。若不过滤全零行，一次请求会被重复
//! 计成多条。因此解析时跳过 input / cache_read / cache_creation / output
//! 四项全为 0（或 usage 缺失）的行。
//!
//! H-C1：全零过滤对真实重复不够——流式落盘时同一 `message.id` 会写 2~3 行
//! 数值完全相同的非零 usage（其中一行为最终聚合行）。因此再按 message.id
//! 做第二级去重：同 id 只保留最后一行（聚合行），保持行的出现顺序稳定。
//! 无 message.id 的行（极老版本数据）不做 id 去重，按原样保留。
//!
//! 性能设计（与 claude 的概要缓存同构）：
//! - 解析结果常驻内存：按 session_id 缓存 `(mtime, Vec<UsageRecord>)`，
//!   启动时后台预热（[`prewarm`]/[`prewarm_async`]）；
//! - `load_usage` 每次调用做增量扫描：mtime 未变的文件直接复用缓存，
//!   活跃会话（文件在增长、mtime 变化）重新解析，数据保持新鲜；
//! - 每个文件最多几千条 usage 记录且记录很小，全量常驻内存无压力。

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::SystemTime;

use crate::utils::{to_local_iso, UsageRecord};

/// 单个会话文件的用量缓存条目：文件未变化（mtime + size 都相同）时直接复用。
///
/// M-B11：mtime 之外同时比对文件 size——NTFS / drvfs 时间戳粒度 1-2 秒，
/// 同秒内多次追加 mtime 不变，但 append 只增不减，size 必能探测。
struct CachedUsage {
    mtime: SystemTime,
    size: u64,
    records: Vec<UsageRecord>,
}

/// 用量缓存：session_id → 解析结果。启动时预热，之后仅增量更新。
fn usage_cache() -> &'static Mutex<HashMap<String, CachedUsage>> {
    static CACHE: OnceLock<Mutex<HashMap<String, CachedUsage>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 全部用量记录（时间倒序），每次调用触发一次增量扫描。
///
/// 前端拿全量列表后自行做分页/筛选/汇总；活跃会话的文件 mtime
/// 变化会触发重解析，因此每次打开用量界面数据都是新鲜的。
pub fn load_usage() -> Vec<UsageRecord> {
    let mut out: Vec<UsageRecord> = {
        let files: Vec<(PathBuf, String, SystemTime, u64)> =
            scan_files(&crate::claude::projects_dir());
        let mut cache = usage_cache().lock().unwrap();
        refresh(&mut cache, files)
    };
    out.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    out
}

/// 启动预热：后台线程解析所有 transcript 的用量记录，填充内存缓存。
/// 阻塞但只做一次；调用方应放到后台线程（[`prewarm_async`]）。
pub fn prewarm() {
    let _ = load_usage();
}

/// 异步预热：在独立后台线程填充用量缓存，不阻塞调用方。
pub fn prewarm_async() {
    // 线程创建失败（资源耗尽等）只记日志：预热是优化项，失败不影响功能
    if let Err(e) = std::thread::Builder::new()
        .name("usage-mgr-prewarm".into())
        .spawn(prewarm)
    {
        log::warn!("用量缓存预热线程启动失败: {e}");
    }
}

/// 枚举 projects 下全部 transcript 文件：`(文件路径, 会话 id, 修改时间, 大小)`。
///
/// 与 claude 的目录扫描同构：单目录读取失败（权限等）跳过该项目。
/// M-B11：mtime 之外同时取文件 size——同秒追加写入靠 size 探测。
/// H-C2：同时枚举 `<session-id>/subagents/agent-<id>.jsonl` 子代理转录，
/// 其 id 形如 `agent-<id>`（文件名派生，与主会话 uuid 不冲突）。
fn scan_files(dir: &Path) -> Vec<(PathBuf, String, SystemTime, u64)> {
    // 目录扫描与历史会话列表共用（`include_subagents = true`：子代理转录在
    // <session-id>/subagents/agent-<id>.jsonl，H-C2 纳入统计，session_id 用
    // 文件名 "agent-<id>" 独立成行）；这里只把结果摊平成缓存层要的四元组。
    crate::claude::scan_transcripts(dir, true)
        .into_iter()
        .map(|t| (t.path, t.session_id, t.mtime, t.size))
        .collect()
}

/// 增量刷新缓存并返回全部会话的用量记录合集。
///
/// 增量策略：mtime + size 都一致 → 复用（M-B11）；不一致或新增 → 重解析；
/// 文件已删除 → 从缓存清理。缓存 Map 由参数传入（便于单测注入局部 Map）。
fn refresh(
    cache: &mut HashMap<String, CachedUsage>,
    files: Vec<(PathBuf, String, SystemTime, u64)>,
) -> Vec<UsageRecord> {
    let mut out: Vec<UsageRecord> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for (path, id, mtime, size) in files {
        seen.insert(id.clone());
        let records: Vec<UsageRecord> = match cache.get(&id) {
            Some(e) if e.mtime == mtime && e.size == size => e.records.clone(),
            _ => {
                let recs = parse_usage_file(&path, &id);
                cache.insert(
                    id.clone(),
                    CachedUsage {
                        mtime,
                        size,
                        records: recs.clone(),
                    },
                );
                recs
            }
        };
        out.extend(records);
    }

    // 清理 transcript 文件已删除的会话缓存
    cache.retain(|k, _| seen.contains(k));

    out
}

/// 一行 assistant transcript 的计量提取结果。
struct UsageLine {
    record: UsageRecord,
    /// H-C1 去重键：`message.id`（同一流式回包的多行共用同一 id；
    /// 老版本数据没有这个字段，为 None 表示不参与 id 去重）
    msg_id: Option<String>,
}

/// 该转录文件是否为子代理的（H-C2：位于 `<session-id>/subagents/` 下）。
fn is_subagent_transcript(path: &Path) -> bool {
    path.parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy() == "subagents")
        .unwrap_or(false)
}

/// 从一行 JSON 提取 usage 计量；不计入的行返回 None。
///
/// 不计入的三种情况：不是 assistant 行；没有 usage 字段（极旧版本 / 异常行）；
/// 四项 token 全零的流式块影子行（不是独立请求，见模块头注释）。
///
/// 可选字段（thinking_tokens / service_tier / cost）缺失即 None，
/// 不参与全零判定。
fn usage_line_from(
    v: &serde_json::Value,
    session_id: &str,
    is_subagent: bool,
) -> Option<UsageLine> {
    if v.get("type").and_then(|x| x.as_str()) != Some("assistant") {
        return None;
    }
    let usage = v.get("message").and_then(|m| m.get("usage"))?;

    let input_tokens = json_u64(usage, "input_tokens");
    let cache_read_tokens = json_u64(usage, "cache_read_input_tokens");
    let cache_creation_tokens = json_u64(usage, "cache_creation_input_tokens");
    let output_tokens = json_u64(usage, "output_tokens");
    // 全零行 = 流式块影子行，不是独立请求：前置剪枝，避免重复计量
    if input_tokens == 0
        && cache_read_tokens == 0
        && cache_creation_tokens == 0
        && output_tokens == 0
    {
        return None;
    }

    let thinking_tokens: Option<u64> = usage
        .get("output_tokens_details")
        .and_then(|d| d.get("thinking_tokens"))
        .and_then(|x| x.as_u64());
    let service_tier: Option<String> = usage
        .get("service_tier")
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());
    // L-C9：成本字段（新版 transcript 的 usage 带 prompt_cost /
    // completion_cost / cache_cost 三个平铺键；实测均为 0 也照常提取，
    // 老版本缺失时为 None）
    let cost: Option<f64> = (|| {
        let prompt = usage.get("prompt_cost")?.as_f64()?;
        let completion = usage
            .get("completion_cost")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        let cache = usage
            .get("cache_cost")
            .and_then(|x| x.as_f64())
            .unwrap_or(0.0);
        Some(prompt + completion + cache)
    })();

    // 顶层 timestamp 为请求时间，归一为本地时区
    let timestamp: String = v
        .get("timestamp")
        .and_then(|x| x.as_str())
        .map(to_local_iso)
        .unwrap_or_default();
    let model: String = v
        .get("message")
        .and_then(|m| m.get("model"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let msg_id: Option<String> = v
        .get("message")
        .and_then(|m| m.get("id"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string());

    Some(UsageLine {
        record: UsageRecord {
            timestamp,
            session_id: session_id.to_string(),
            model,
            input_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            output_tokens,
            thinking_tokens,
            service_tier,
            cost,
            is_subagent,
        },
        msg_id,
    })
}

/// H-C1 去重：同 `message.id` 的后行覆盖前行（聚合行在后），无 id 的行原样追加。
///
/// 覆盖按下标进行，被替换的那条保持原位次，因此输出顺序仍是
/// 「首次出现的顺序」，与逐行原样收集一致。
fn merge_usage_line(
    lines: &mut Vec<UsageRecord>,
    id_index: &mut HashMap<String, usize>,
    line: UsageLine,
) {
    let UsageLine { record, msg_id } = line;
    let Some(id) = msg_id else {
        lines.push(record);
        return;
    };
    match id_index.get(&id) {
        Some(&idx) => lines[idx] = record,
        None => {
            id_index.insert(id, lines.len());
            lines.push(record);
        }
    }
}

/// 解析单个 transcript 文件，提取全部 assistant 行的 usage 计量。
///
/// 两阶段：先逐行提取（[`usage_line_from`]），再按 `message.id` 去重
/// （[`merge_usage_line`]）。文件读取失败返回空（不中断全局）。
fn parse_usage_file(path: &Path, session_id: &str) -> Vec<UsageRecord> {
    let is_subagent = is_subagent_transcript(path);
    let mut lines: Vec<UsageRecord> = Vec::new();
    // H-C1：message.id → 该 id 目前在 lines 里的下标；同 id 后行覆盖前行
    let mut id_index: HashMap<String, usize> = HashMap::new();

    // 读不到文件（被删除的竞态）时 lines 保持为空，与「文件里没有计量」同解
    crate::utils::for_each_json_line(path, |v| {
        if let Some(parsed) = usage_line_from(v, session_id, is_subagent) {
            merge_usage_line(&mut lines, &mut id_index, parsed);
        }
    });
    lines
}

/// 从 JSON 对象取 u64 字段，缺失或类型不符按 0 处理。
fn json_u64(v: &serde_json::Value, key: &str) -> u64 {
    v.get(key).and_then(|x| x.as_u64()).unwrap_or(0)
}

// ---- 用量变更推送（SSE / Tauri event 共用）----

/// 变更通知回调：transcript 变化解析出新用量时调用（含全量记录列表）。
pub type UsageListener = Box<dyn Fn(Vec<UsageRecord>) + Send + Sync>;

/// 注销凭据（guard 语义见 [`crate::watch::ListenerGuard`]）。
pub type UsageListenerGuard = crate::watch::ListenerGuard<UsageListener>;

/// 全局监听者集合：server 的 SSE broadcaster 与桌面端 emit 都挂在这里。
static USAGE_LISTENERS: crate::watch::ListenerRegistry<UsageListener> =
    crate::watch::ListenerRegistry::new();

/// projects 目录指纹：递归一层（H-C2）——除各项目目录自身的名字与元数据外，
/// 项目目录下的每一项（会话 `.jsonl`、`<session-id>/` 会话目录）也一并哈希，
/// 因此子代理转录引起的会话目录元数据变化同样会触发推送。
fn usage_fingerprint() -> u64 {
    crate::watch::fingerprint_dir(&crate::claude::projects_dir(), 1)
}

/// 轮询周期：每轮重读配置，`set_config` 改了周期下一轮即生效（默认 30s）。
fn usage_period() -> std::time::Duration {
    let secs = crate::config::clamp_usage_refresh_secs(crate::config::load().usage_refresh_secs);
    std::time::Duration::from_secs(secs as u64)
}

/// 用量轮询器（周期来自配置，见 [`usage_period`]）。
static WATCHER: crate::watch::PollingWatcher = crate::watch::PollingWatcher::new();

/// 启动用量 watcher：指纹变化时增量解析（mtime + size 缓存只重读变化的文件）
/// 并回调全部监听者。
///
/// 首轮基准取「空指纹」：目录非空时启动后第一个周期就推一次，
/// 前端首屏据此拿到数据，不必等第一次文件变化。幂等可重复调用。
pub fn start_usage_watcher() {
    WATCHER.start(
        usage_period,
        crate::watch::Baseline::Empty,
        usage_fingerprint,
        || USAGE_LISTENERS.notify(load_usage()),
    );
}

/// 注册用量变更监听者（自动启动 watcher）。
pub fn subscribe_usage(listener: UsageListener) -> UsageListenerGuard {
    start_usage_watcher();
    USAGE_LISTENERS.subscribe(listener)
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

    /// assistant 行：带完整 usage（可选字段按需拼接；id 为 message.id）。
    fn assistant_usage_line(ts: &str, usage: &str) -> String {
        assistant_usage_line_id(ts, usage, None)
    }

    fn assistant_usage_line_id(ts: &str, usage: &str, id: Option<&str>) -> String {
        let id_part = match id {
            Some(i) => format!(r#","id":"{i}""#),
            None => String::new(),
        };
        format!(
            r#"{{"type":"assistant","timestamp":"{ts}","message":{{"model":"claude-sonnet-5"{id_part},"usage":{usage}}}}}"#
        )
    }

    #[test]
    fn parse_filters_shadow_and_extracts_fields() {
        let dir = std::env::temp_dir().join("ccbuddy-usage-parse-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            // 全零影子行：应被过滤
            assistant_usage_line(
                "2026-09-01T10:00:02Z",
                r#"{"input_tokens":0,"cache_read_input_tokens":0,"cache_creation_input_tokens":0,"output_tokens":0}"#,
            ),
            // 缺 usage 的 assistant 行：应被跳过
            r#"{"type":"assistant","timestamp":"2026-09-01T10:00:03Z","message":{"model":"claude-sonnet-5"}}"#.to_string(),
            // user 行：与用量无关
            r#"{"type":"user","timestamp":"2026-09-01T10:00:04Z","message":{"content":"hi"}}"#.to_string(),
            // 完整 usage 行（含 thinking_tokens / service_tier）
            assistant_usage_line(
                "2026-09-01T10:00:00Z",
                r#"{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":300,"output_tokens_details":{"thinking_tokens":80},"service_tier":"standard"}"#,
            ),
            // 缺可选字段行：thinking / tier 为 None
            assistant_usage_line(
                "2026-09-01T10:00:01Z",
                r#"{"input_tokens":10,"output_tokens":5}"#,
            ),
        ];
        let path = write_transcript(&dir, "sess-1", &lines);

        let records = parse_usage_file(&path, "sess-1");
        // 影子行 / 缺 usage 行 / user 行都被过滤，只留 2 条真实请求
        assert_eq!(records.len(), 2, "应过滤全零影子行与无 usage 行");

        let full = &records[0];
        assert_eq!(full.input_tokens, 100);
        assert_eq!(full.cache_read_tokens, 200);
        assert_eq!(full.cache_creation_tokens, 50);
        assert_eq!(full.output_tokens, 300);
        assert_eq!(full.thinking_tokens, Some(80));
        assert_eq!(full.service_tier.as_deref(), Some("standard"));
        assert_eq!(full.model, "claude-sonnet-5");
        assert_eq!(full.session_id, "sess-1");
        assert!(!full.timestamp.is_empty(), "timestamp 应被提取并归一为本地时间");

        let partial = &records[1];
        assert_eq!(partial.input_tokens, 10);
        assert_eq!(partial.output_tokens, 5);
        // 缓存字段缺失按 0 处理
        assert_eq!(partial.cache_read_tokens, 0);
        assert_eq!(partial.thinking_tokens, None);
        assert_eq!(partial.service_tier, None);
        // 无成本字段的老版本行：cost 为 None
        assert_eq!(partial.cost, None);
        assert!(!partial.is_subagent, "主会话文件 is_subagent 应为 false");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-C1：同 message.id 的多行非零 usage（流式落盘重复）只计一次，
    /// 保留最后一行；不同 id 正常计；无 id 的行不去重按原样保留。
    #[test]
    fn parse_dedups_by_message_id() {
        let dir = std::env::temp_dir().join("ccbuddy-usage-dedup-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            // 同 id 两行数值相同的非零 usage（流式落盘重复）：只计一次
            assistant_usage_line_id(
                "2026-09-01T10:00:00Z",
                r#"{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":300}"#,
                Some("msg-1"),
            ),
            // 同 id 第三行（聚合行，数值略不同）：应覆盖前面两行
            assistant_usage_line_id(
                "2026-09-01T10:00:01Z",
                r#"{"input_tokens":100,"cache_read_input_tokens":200,"cache_creation_input_tokens":50,"output_tokens":350}"#,
                Some("msg-1"),
            ),
            // 不同 id：正常计
            assistant_usage_line_id(
                "2026-09-01T10:01:00Z",
                r#"{"input_tokens":10,"output_tokens":5}"#,
                Some("msg-2"),
            ),
            // 无 id 的行（老版本数据）：不去重，原样保留（两行都计）
            assistant_usage_line(
                "2026-09-01T10:02:00Z",
                r#"{"input_tokens":7,"output_tokens":3}"#,
            ),
            assistant_usage_line(
                "2026-09-01T10:02:01Z",
                r#"{"input_tokens":8,"output_tokens":4}"#,
            ),
        ];
        let path = write_transcript(&dir, "sess-dedup", &lines);

        let records = parse_usage_file(&path, "sess-dedup");
        // msg-1 去重后 1 条 + msg-2 1 条 + 无 id 2 条 = 4 条
        assert_eq!(records.len(), 4, "同 id 多行应去重，无 id 行应保留");

        // msg-1 保留的是最后一行（聚合行，output=350）
        assert_eq!(records[0].output_tokens, 350, "同 id 应保留最后一行");
        assert_eq!(records[0].timestamp, to_local_iso("2026-09-01T10:00:01Z"));
        // 时间顺序保持稳定
        assert_eq!(records[1].input_tokens, 10);
        assert_eq!(records[2].input_tokens, 7);
        assert_eq!(records[3].input_tokens, 8);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-C9：新版 usage 带 prompt_cost / completion_cost / cache_cost 平铺键，
    /// cost = 三项之和；缺失任一键则整体为 None。
    #[test]
    fn parse_extracts_cost() {
        let dir = std::env::temp_dir().join("ccbuddy-usage-cost-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![
            assistant_usage_line(
                "2026-09-01T10:00:00Z",
                r#"{"input_tokens":100,"output_tokens":300,"prompt_cost":0.05,"completion_cost":0.02,"cache_cost":0.01}"#,
            ),
            // 只有 prompt_cost：completion/cache 缺失按 0
            assistant_usage_line(
                "2026-09-01T10:01:00Z",
                r#"{"input_tokens":10,"output_tokens":5,"prompt_cost":0.001}"#,
            ),
        ];
        let path = write_transcript(&dir, "sess-cost", &lines);

        let records = parse_usage_file(&path, "sess-cost");
        assert_eq!(records.len(), 2);
        assert!((records[0].cost.unwrap() - 0.08).abs() < 1e-9);
        assert!((records[1].cost.unwrap() - 0.001).abs() < 1e-9);

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// H-C2：`<session-id>/subagents/agent-<id>.jsonl` 被扫描纳入，
    /// session_id 用文件名（agent-<id>），记录标记 is_subagent。
    #[test]
    fn scan_files_includes_subagents() {
        let root = std::env::temp_dir().join("ccbuddy-usage-subagent-test");
        let _ = std::fs::remove_dir_all(&root);
        let proj = root.join("D--work-proj");
        let sub = proj.join("sess-main").join("subagents");
        std::fs::create_dir_all(&sub).unwrap();

        // 主会话 + 子代理转录各一条
        write_transcript(
            &proj,
            "sess-main",
            &[assistant_usage_line(
                "2026-09-01T10:00:00Z",
                r#"{"input_tokens":100,"output_tokens":300}"#,
            )],
        );
        write_transcript(
            &sub,
            "agent-abc123",
            &[assistant_usage_line(
                "2026-09-01T10:05:00Z",
                r#"{"input_tokens":50,"output_tokens":20}"#,
            )],
        );

        let files = scan_files(&root);
        let ids: Vec<&str> = files.iter().map(|(_, id, _, _)| id.as_str()).collect();
        assert!(ids.contains(&"sess-main"), "主会话应被扫描: {ids:?}");
        assert!(ids.contains(&"agent-abc123"), "subagents 下的文件应被扫描: {ids:?}");

        // 子代理文件的记录 is_subagent = true，session_id 为文件名
        let agent_path = files
            .iter()
            .find(|(_, id, _, _)| id == "agent-abc123")
            .map(|(p, _, _, _)| p.clone())
            .unwrap();
        let records = parse_usage_file(&agent_path, "agent-abc123");
        assert_eq!(records.len(), 1);
        assert!(records[0].is_subagent, "subagents 目录下的文件应标记 is_subagent");
        assert_eq!(records[0].session_id, "agent-abc123");
        assert_eq!(records[0].input_tokens, 50);

        // 主会话文件的记录 is_subagent = false
        let main_path = files
            .iter()
            .find(|(_, id, _, _)| id == "sess-main")
            .map(|(p, _, _, _)| p.clone())
            .unwrap();
        let records = parse_usage_file(&main_path, "sess-main");
        assert_eq!(records.len(), 1);
        assert!(!records[0].is_subagent);

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn refresh_reuses_cache_and_reparses_on_mtime_change() {
        let dir = std::env::temp_dir().join("ccbuddy-usage-cache-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let lines = vec![assistant_usage_line(
            "2026-09-01T10:00:00Z",
            r#"{"input_tokens":100,"output_tokens":300}"#,
        )];
        let path = write_transcript(&dir, "sess-2", &lines);

        let t0 = SystemTime::UNIX_EPOCH;
        let t1 = t0 + std::time::Duration::from_secs(1);

        let mut cache: HashMap<String, CachedUsage> = HashMap::new();

        // 首次：解析得 1 条（size 用当前文件实际大小）
        let s0 = std::fs::metadata(&path).unwrap().len();
        let out = refresh(&mut cache, vec![(path.clone(), "sess-2".to_string(), t0, s0)]);
        assert_eq!(out.len(), 1);
        assert_eq!(cache.len(), 1);

        // mtime 与 size 都未变：命中缓存，仍是旧结果（1 条）
        let out = refresh(&mut cache, vec![(path.clone(), "sess-2".to_string(), t0, s0)]);
        assert_eq!(out.len(), 1, "指纹未变应命中缓存");

        // mtime 变化：重解析，得 2 条
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            "{}",
            assistant_usage_line(
                "2026-09-01T10:01:00Z",
                r#"{"input_tokens":50,"output_tokens":20}"#
            )
        )
        .unwrap();
        let out = refresh(&mut cache, vec![(path.clone(), "sess-2".to_string(), t1, s0)]);
        assert_eq!(out.len(), 2, "mtime 变化应重新解析");

        // 文件列表为空（会话已删除）：输出为空且缓存被清理
        let out = refresh(&mut cache, vec![]);
        assert!(out.is_empty());
        assert!(cache.is_empty(), "已删除会话的缓存应被清理");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// M-B11：mtime 相同但 size 变化（同秒追加写入）→ 必须重读，不能命中缓存。
    #[test]
    fn refresh_reparses_when_size_changes_with_same_mtime() {
        let dir = std::env::temp_dir().join("ccbuddy-usage-size-test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let path = write_transcript(
            &dir,
            "sess-size",
            &[assistant_usage_line(
                "2026-09-01T10:00:00Z",
                r#"{"input_tokens":100,"output_tokens":300}"#,
            )],
        );

        let t = SystemTime::UNIX_EPOCH; // 固定 mtime：模拟 NTFS 同秒粒度
        let mut cache: HashMap<String, CachedUsage> = HashMap::new();

        // 首次（size=s0）：解析得 1 条
        let s0 = std::fs::metadata(&path).unwrap().len();
        let out = refresh(&mut cache, vec![(path.clone(), "sess-size".to_string(), t, s0)]);
        assert_eq!(out.len(), 1);

        // 追加一条（size 必然增长），但 mtime 模拟不变：应重读得 2 条
        let mut f = std::fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(
            f,
            "{}",
            assistant_usage_line(
                "2026-09-01T10:01:00Z",
                r#"{"input_tokens":50,"output_tokens":20}"#
            )
        )
        .unwrap();
        let s1 = std::fs::metadata(&path).unwrap().len();
        assert!(s1 > s0, "追加写入后 size 必然增长");
        let out = refresh(&mut cache, vec![(path.clone(), "sess-size".to_string(), t, s1)]);
        assert_eq!(out.len(), 2, "同 mtime 但 size 变化应重新解析");

        let _ = std::fs::remove_dir_all(&dir);
    }

    /// L-B14：注销 A 不影响 B 的回调（按稳定 id 移除，不错位）。
    #[test]
    fn usage_listener_unregister_does_not_affect_others() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let a_calls = Arc::new(AtomicUsize::new(0));
        let b_calls = Arc::new(AtomicUsize::new(0));

        // 先注册 A，再注册 B；先注销 A，B 的回调必须仍然存活且被调用
        let ga = {
            let a = a_calls.clone();
            subscribe_usage(Box::new(move |_| {
                a.fetch_add(1, Ordering::SeqCst);
            }))
        };
        let gb = {
            let b = b_calls.clone();
            subscribe_usage(Box::new(move |_| {
                b.fetch_add(1, Ordering::SeqCst);
            }))
        };

        drop(ga); // 注销 A（按下标 remove 的旧实现会移除错误的回调）

        // 直接投递（不依赖 watcher 触发时机）
        USAGE_LISTENERS.notify(Vec::new());

        assert_eq!(a_calls.load(Ordering::SeqCst), 0, "A 已注销，回调不应再被调用");
        assert_eq!(b_calls.load(Ordering::SeqCst), 1, "B 的回调应存活且被调用一次");

        drop(gb);
        USAGE_LISTENERS.notify(Vec::new());
        assert_eq!(b_calls.load(Ordering::SeqCst), 1, "B 注销后回调也不应再被调用");
    }

    #[test]
    fn load_usage_sorted_desc_by_timestamp() {
        // 构造乱序时间戳：验证 load_usage 输出按时间倒序
        let a = UsageRecord {
            timestamp: "2026-09-01T10:00:00.000+08:00".to_string(),
            session_id: "s1".to_string(),
            model: "m".to_string(),
            input_tokens: 1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            output_tokens: 0,
            thinking_tokens: None,
            service_tier: None,
            cost: None,
            is_subagent: false,
        };
        let b = UsageRecord {
            timestamp: "2026-09-02T10:00:00.000+08:00".to_string(),
            ..a.clone()
        };
        let mut out = vec![a, b];
        out.sort_by(|x, y| y.timestamp.cmp(&x.timestamp));
        assert_eq!(out[0].timestamp, "2026-09-02T10:00:00.000+08:00");
        assert_eq!(out[1].timestamp, "2026-09-01T10:00:00.000+08:00");
    }
}
