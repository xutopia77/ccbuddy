//! 对外行为基线：从公开入口 `core::dispatch` 出发，固定后端 RPC 命令的对外契约。
//!
//! 它只依赖对外可见的东西——命令名、状态码、JSON 字段名与关键取值，
//! 不引用任何内部函数名。因此它是「结构层重构没有改变行为」的硬证据：
//! 模块拆分 / 私有函数改名后，本文件必须**原样通过**。
//!
//! 放在 `tests/` 下意味着独立的可执行文件 = 独立进程，可以放心用
//! `CCBUDDY_DATA_ROOT` 把数据根目录指向临时目录（不影响同进程的其他测试）。
//! 断言全部写在单个 `#[test]` 内顺序执行：进程级环境变量与全局缓存的
//! 语义要求串行，且失败时能一次看到完整链路。
//!
//! 覆盖范围：get_events / get_sessions / get_transcript_retention / get_usage /
//! get_event_detail / get_session_detail / get_config / set_config /
//! get_hook_status，以及未知命令与参数校验的错误路径。
//! 有意不覆盖 install_hooks / uninstall_hooks：它们依赖内嵌的 hook 构建产物，
//! 属于「安装器」而非「查询行为」，命令行手工验证更合适。

use std::path::{Path, PathBuf};

use ccbuddy_lib::core::{dispatch, RpcContext};
use ccbuddy_lib::RpcResponse;
use serde_json::{json, Value};

/// 事件流会话 id（同时是 `event-<id>.jsonl` 的文件名主体）。
const EVENTS_ID: &str = "sess-events-1";
/// 原生 transcript 会话 id（`<claude_dir>/projects/<项目>/<id>.jsonl`）。
const NATIVE_ID: &str = "sess-native-1";
/// 项目目录名（真实环境是 cwd 的编码，这里只用名字本身，与解析无关）。
const PROJECT_DIR: &str = "D--work-proj";
/// transcript / hook 事件里的工作目录（项目名取最后一段 → "proj"）。
const CWD: &str = "D:/work/proj";
/// 用户首条输入：同时用作标题回退与消息内容。
const PROMPT: &str = "帮我修复登录问题";
/// 会话 agentName（`type:"agent-name"` 行）：标题优先级高于首条输入。
const AGENT_NAME: &str = "登录修复会话";
/// assistant 文本回复：同时用作列表预览。
const REPLY: &str = "好的，我来看看";
const MODEL: &str = "claude-sonnet-5";

// ---- 断言辅助 ----

/// 成功响应信封：code=0、status="ok"、cmd 回显、time 为 UTC 毫秒格式，返回 data。
fn ok_data<'a>(r: &'a RpcResponse, cmd: &str) -> &'a Value {
    assert_eq!(r.code, 0, "{cmd} 应成功，实际 status={}", r.status);
    assert_eq!(r.status, "ok");
    assert_eq!(r.cmd, cmd, "cmd 必须原样回显");
    assert_eq!(r.time.len(), 24, "time 应为 UTC 毫秒格式: {}", r.time);
    assert!(r.time.ends_with('Z'), "time 应以 Z 结尾: {}", r.time);
    &r.data
}

/// 失败响应信封：code 匹配、cmd 回显、data 恒为 null、status 为错误说明。
fn assert_err(r: &RpcResponse, cmd: &str, code: i32) {
    assert_eq!(r.code, code, "{cmd} 应失败 code={code}，实际 status={}", r.status);
    assert_eq!(r.cmd, cmd);
    assert_eq!(r.data, Value::Null, "失败响应 data 恒为 null");
    assert_ne!(r.status, "ok", "失败响应 status 不得为 ok");
}

/// JSON 对象的键名（排序后），用于锁死字段名集合。
fn keys(v: &Value) -> Vec<String> {
    let mut k: Vec<String> = v.as_object().expect("应为 JSON 对象").keys().cloned().collect();
    k.sort();
    k
}

fn expected_keys(list: &[&str]) -> Vec<String> {
    let mut v: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    v.sort();
    v
}

/// 本地时区 ISO（`to_local_iso`）形状：日期时间 + 偏移，如 2026-09-01T18:00:03.000+08:00。
/// 不断言具体日期/偏移（随运行机器时区变化），只锁格式。
fn assert_local_iso(s: &str, what: &str) {
    assert_eq!(s.len(), 29, "{what} 应为带偏移的本地 ISO: {s}");
    assert!(s.contains('T'), "{what} 应含日期与时间分隔符: {s}");
}

/// 消息行时间（`short_time`）形状："YYYY-MM-DD HH:MM:SS"。
fn assert_short_time(s: &str, what: &str) {
    assert_eq!(s.len(), 19, "{what} 应为 YYYY-MM-DD HH:MM:SS: {s}");
}

fn write(path: &Path, text: &str) {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

/// 把若干 JSON 值写成 jsonl 文本。
fn jsonl(lines: &[Value]) -> String {
    let mut s = lines.iter().map(|v| v.to_string()).collect::<Vec<_>>().join("\n");
    s.push('\n');
    s
}

/// 准备临时数据根目录与合成数据，返回 (数据根, claude 目录)。
fn fixture() -> (PathBuf, PathBuf) {
    let root = std::env::temp_dir().join("ccbuddy-dispatch-baseline");
    let _ = std::fs::remove_dir_all(&root);
    let claude = root.join("claude");
    std::fs::create_dir_all(&claude).unwrap();

    // 数据根目录覆盖：必须在任何配置读取之前设置
    std::env::set_var("CCBUDDY_DATA_ROOT", &root);

    // hook 事件流：SessionStart → UserPromptSubmit → PreToolUse → Stop
    write(
        &root.join("events").join(format!("event-{EVENTS_ID}.jsonl")),
        &jsonl(&[
            json!({"received_at": "2026-09-01T10:00:00Z", "hook_event": "SessionStart",
                   "payload": {"session_id": EVENTS_ID, "cwd": CWD, "source": "startup"}}),
            json!({"received_at": "2026-09-01T10:00:01Z", "hook_event": "UserPromptSubmit",
                   "payload": {"session_id": EVENTS_ID, "cwd": CWD, "prompt": PROMPT}}),
            json!({"received_at": "2026-09-01T10:00:02Z", "hook_event": "PreToolUse",
                   "payload": {"session_id": EVENTS_ID, "cwd": CWD, "tool_name": "Read",
                               "tool_input": {"file_path": "src/main.rs"}}}),
            // 无工具调用的普通事件：既有真实日志形状，又补一条带 usage 的行
            json!({"received_at": "2026-09-01T10:00:03Z", "hook_event": "MessageDisplay",
                   "payload": {"session_id": EVENTS_ID, "cwd": CWD, "message": REPLY}}),
            json!({"received_at": "2026-09-01T10:00:04Z", "hook_event": "Stop",
                   "payload": {"session_id": EVENTS_ID, "cwd": CWD}}),
        ]),
    );

    // 原生 transcript：用户首条输入 + agentName + assistant 回复（带 usage）
    write(
        &claude.join("projects").join(PROJECT_DIR).join(format!("{NATIVE_ID}.jsonl")),
        &jsonl(&[
            json!({"type": "user", "cwd": CWD, "timestamp": "2026-09-01T09:00:00Z",
                   "message": {"content": PROMPT}}),
            json!({"type": "agent-name", "agentName": AGENT_NAME}),
            json!({"type": "assistant", "timestamp": "2026-09-01T09:00:05Z",
                   "message": {"id": "msg_1", "model": MODEL,
                               "content": [{"type": "text", "text": REPLY}],
                               "usage": {"input_tokens": 100, "cache_read_input_tokens": 20,
                                         "cache_creation_input_tokens": 5, "output_tokens": 50}}}),
        ]),
    );

    // settings.json：保留期（只读展示）+ 空的 hooks 段（hook 状态对账用）
    write(
        &claude.join("settings.json"),
        &json!({"cleanupPeriodDays": 30, "hooks": {}}).to_string(),
    );

    // 应用配置：claude 目录指向临时目录
    write(
        &root.join("config.json"),
        &json!({"claude_dir": claude.to_string_lossy(), "usage_refresh_secs": 30}).to_string(),
    );

    (root, claude)
}

#[test]
fn dispatch_对外行为基线() {
    let (root, claude) = fixture();
    let ctx = RpcContext::default();

    // ---- get_events：事件流会话列表（含消息，标题取首条用户输入） ----
    let r = dispatch(&ctx, "get_events", Value::Null);
    let list = ok_data(&r, "get_events").as_array().unwrap().clone();
    assert_eq!(list.len(), 1, "只应有一个事件流会话");
    let ev = &list[0];
    assert_eq!(
        keys(ev),
        expected_keys(&[
            "id", "project", "cwd", "title", "status", "lastActivity", "preview", "unread",
            "messages", "source",
        ])
    );
    assert_eq!(ev["id"], EVENTS_ID);
    assert_eq!(ev["project"], "proj");
    assert_eq!(ev["cwd"], CWD);
    assert_eq!(ev["title"], PROMPT);
    assert_eq!(ev["status"], "completed", "Stop 后应为 completed");
    assert_eq!(ev["unread"], false);
    assert_eq!(ev["source"], "startup", "SessionStart 的 source 应回传");
    assert_local_iso(ev["lastActivity"].as_str().unwrap(), "lastActivity");

    let msgs = ev["messages"].as_array().unwrap();
    // UserPromptSubmit 一条 user + PreToolUse 一条 assistant + MessageDisplay 一条 assistant
    assert_eq!(msgs.len(), 3);
    assert_eq!(keys(&msgs[0]), expected_keys(&["type", "role", "content", "time"]));
    assert_eq!(msgs[0]["type"], "user");
    assert_eq!(msgs[0]["role"], "user");
    assert_eq!(msgs[0]["content"], PROMPT);
    assert_short_time(msgs[0]["time"].as_str().unwrap(), "消息时间");
    assert_eq!(msgs[1]["type"], "assistant");
    // 工具行内容：首行"调用工具 X"，其后为入参的一行概要（前端拆行后直接展示）
    assert_eq!(msgs[1]["content"], "调用工具 Read\nsrc/main.rs");
    assert_eq!(msgs[1]["toolCall"], "Read");
    assert_eq!(
        keys(&msgs[1]),
        expected_keys(&["type", "role", "content", "time", "toolCall"])
    );
    assert_eq!(msgs[2]["content"], REPLY);
    // 预览取最近一条有内容的事件（MessageDisplay 的文本）
    assert_eq!(ev["preview"], REPLY);

    // ---- get_event_detail：同一会话的详情，字段与列表一致 ----
    let r = dispatch(&ctx, "get_event_detail", json!(EVENTS_ID));
    let detail = ok_data(&r, "get_event_detail");
    assert_eq!(detail["id"], EVENTS_ID);
    assert_eq!(detail["messages"].as_array().unwrap().len(), 3);
    // 会话不存在 → 404；id 类型不对 → 400
    assert_err(&dispatch(&ctx, "get_event_detail", json!("no-such-session")), "get_event_detail", 404);
    assert_err(&dispatch(&ctx, "get_event_detail", json!(123)), "get_event_detail", 400);

    // ---- get_sessions：原生 transcript 会话列表（懒加载，不含消息体） ----
    let r = dispatch(&ctx, "get_sessions", Value::Null);
    let list = ok_data(&r, "get_sessions").as_array().unwrap().clone();
    assert_eq!(list.len(), 1);
    let s = &list[0];
    assert_eq!(
        keys(s),
        expected_keys(&[
            "id", "project", "cwd", "title", "status", "lastActivity", "preview", "unread",
            "messages",
        ]),
        "无 source/transcriptPath/subagents 的会话不应出现这些键"
    );
    assert_eq!(s["id"], NATIVE_ID);
    assert_eq!(s["project"], "proj");
    assert_eq!(s["cwd"], CWD);
    assert_eq!(s["title"], AGENT_NAME, "标题优先级：agentName > 首条用户输入");
    assert_eq!(s["status"], "completed");
    assert_eq!(s["preview"], REPLY);
    assert_eq!(s["unread"], false);
    assert!(s["messages"].as_array().unwrap().is_empty(), "列表查询为懒加载");
    assert_local_iso(s["lastActivity"].as_str().unwrap(), "lastActivity");

    // ---- get_session_detail：详情解析全量消息 ----
    let r = dispatch(&ctx, "get_session_detail", json!(NATIVE_ID));
    let detail = ok_data(&r, "get_session_detail");
    assert_eq!(detail["id"], NATIVE_ID);
    assert_eq!(detail["title"], AGENT_NAME);
    let msgs = detail["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 2, "user + assistant，agent-name 行不入消息流");
    assert_eq!(msgs[0]["type"], "user");
    assert_eq!(msgs[0]["content"], PROMPT);
    assert_eq!(msgs[1]["type"], "assistant");
    assert_eq!(msgs[1]["role"], "assistant");
    assert_eq!(msgs[1]["content"], REPLY);
    assert!(msgs[1].get("toolCall").is_none());
    assert_err(&dispatch(&ctx, "get_session_detail", json!("no-such-session")), "get_session_detail", 404);
    assert_err(&dispatch(&ctx, "get_session_detail", json!([])), "get_session_detail", 400);

    // ---- get_transcript_retention：只读 settings.json 的 cleanupPeriodDays ----
    let r = dispatch(&ctx, "get_transcript_retention", Value::Null);
    assert_eq!(ok_data(&r, "get_transcript_retention"), &json!(30));

    // ---- get_usage：token 计量 ----
    let r = dispatch(&ctx, "get_usage", Value::Null);
    let list = ok_data(&r, "get_usage").as_array().unwrap().clone();
    assert_eq!(list.len(), 1);
    let u = &list[0];
    assert_eq!(
        keys(u),
        expected_keys(&[
            "timestamp", "sessionId", "model", "inputTokens", "cacheReadTokens",
            "cacheCreationTokens", "outputTokens", "isSubagent",
        ]),
        "可选字段（thinkingTokens/serviceTier/cost）缺失时不序列化"
    );
    assert_eq!(u["sessionId"], NATIVE_ID);
    assert_eq!(u["model"], MODEL);
    assert_eq!(u["inputTokens"], 100);
    assert_eq!(u["cacheReadTokens"], 20);
    assert_eq!(u["cacheCreationTokens"], 5);
    assert_eq!(u["outputTokens"], 50);
    assert_eq!(u["isSubagent"], false);
    assert_local_iso(u["timestamp"].as_str().unwrap(), "用量时间");

    // ---- get_config：用户配置 + 只读派生字段 ----
    let r = dispatch(&ctx, "get_config", Value::Null);
    let cfg = ok_data(&r, "get_config");
    assert_eq!(
        keys(cfg),
        expected_keys(&["claude_dir", "usage_refresh_secs", "events_dir", "data_root", "log_level"])
    );
    assert_eq!(cfg["claude_dir"], claude.to_string_lossy().as_ref());
    assert_eq!(cfg["usage_refresh_secs"], 30);
    assert_eq!(cfg["events_dir"], root.join("events").to_string_lossy().as_ref());
    assert_eq!(cfg["data_root"], root.to_string_lossy().as_ref());
    assert!(cfg["log_level"].is_string());

    // ---- set_config：部分更新，返回更新后的配置视图 ----
    let r = dispatch(&ctx, "set_config", json!({"usage_refresh_secs": "60"}));
    let cfg = ok_data(&r, "set_config");
    assert_eq!(cfg["usage_refresh_secs"], 60);
    // 超出区间被钳制到上限
    let r = dispatch(&ctx, "set_config", json!({"usage_refresh_secs": "99999"}));
    assert_eq!(ok_data(&r, "set_config")["usage_refresh_secs"], 3600);
    // 未知/只读字段 → 内部错误；patch 不是对象 → 参数错误
    assert_err(&dispatch(&ctx, "set_config", json!({"nope": "x"})), "set_config", 500);
    assert_err(&dispatch(&ctx, "set_config", json!({"usage_refresh_secs": 60})), "set_config", 500);
    assert_err(&dispatch(&ctx, "set_config", json!("nope")), "set_config", 400);

    // ---- get_hook_status：安装/注册状态 ----
    let r = dispatch(&ctx, "get_hook_status", Value::Null);
    let st = ok_data(&r, "get_hook_status");
    assert_eq!(keys(st), expected_keys(&["installed", "registered", "broken", "broken_reason"]));
    assert_eq!(st["installed"], false, "临时 claude 目录下没有 hook 文件");
    assert_eq!(st["broken"], false);
    assert_eq!(st["broken_reason"], Value::Null);
    let registered = st["registered"].as_object().unwrap();
    assert_eq!(registered.len(), ccbuddy_lib::HOOK_EVENTS.len(), "注册状态应覆盖全部 hook 事件");
    assert!(registered.values().all(|v| v == &Value::Bool(false)), "临时目录下无人注册");

    // ---- 未知命令：参数错误 ----
    assert_err(&dispatch(&ctx, "no_such_cmd", Value::Null), "no_such_cmd", 400);
}
