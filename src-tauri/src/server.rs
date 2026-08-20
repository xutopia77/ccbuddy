//! 内嵌 HTTP 服务（无头 Linux / 浏览器访问场景）。
//!
//! 监听 `127.0.0.1:8787`，提供：
//! - `GET /api/sessions` — 会话列表 JSON
//! - `GET /api/stats`    — 各状态计数
//! - `GET /`             — 极简暗色 Web 界面（前端 fetch 实时渲染）

use std::collections::HashMap;

use axum::response::Html;
use axum::routing::get;
use axum::{Json, Router};
use serde_json::Value;

use crate::state;

pub async fn start() {
    let app = Router::new()
        .route("/api/sessions", get(sessions_api))
        .route("/api/stats", get(stats_api))
        .route("/", get(index))
        .fallback(not_found);

    let listener = match tokio::net::TcpListener::bind("127.0.0.1:8787").await {
        Ok(l) => l,
        Err(e) => {
            eprintln!("[ccbuddy] 无法监听 127.0.0.1:8787（端口可能被占用）: {e}");
            return;
        }
    };
    println!("[ccbuddy] HTTP 服务已启动: http://127.0.0.1:8787");
    if let Err(e) = axum::serve(listener, app).await {
        eprintln!("[ccbuddy] HTTP 服务异常: {e}");
    }
}

async fn sessions_api() -> Json<Vec<state::SessionInfo>> {
    Json(state::load_sessions())
}

async fn stats_api() -> Json<Value> {
    let sessions = state::load_sessions();
    let mut counts: HashMap<String, usize> = HashMap::new();
    for s in &sessions {
        *counts.entry(s.status.clone()).or_insert(0) += 1;
    }
    counts.insert("total".to_string(), sessions.len());
    Json(serde_json::to_value(counts).unwrap_or(Value::Null))
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn not_found() -> Html<&'static str> {
    Html("<h1>404 Not Found</h1>")
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html lang="zh-CN">
<head>
<meta charset="UTF-8" />
<meta name="viewport" content="width=device-width, initial-scale=1.0" />
<title>CCBuddy - Claude Code 会话管理器</title>
<style>
:root{--bg:#0a0f16;--bg2:#111823;--bg3:#1a2332;--border:#263445;--text:#e2e8f0;--muted:#94a3b8;--accent:#3b82f6;--green:#10b981;--orange:#f59e0b;--red:#ef4444;--blue:#3b82f6;--gray:#6b7280}
*{margin:0;padding:0;box-sizing:border-box}
body{font-family:-apple-system,'Segoe UI','PingFang SC','Microsoft YaHei',sans-serif;background:var(--bg);color:var(--text);min-height:100vh}
header{height:52px;background:var(--bg2);border-bottom:1px solid var(--border);display:flex;align-items:center;padding:0 20px;gap:12px;position:sticky;top:0}
.logo{font-weight:700;color:var(--accent)}
.badge{font-size:11px;color:var(--muted);background:var(--bg3);padding:3px 8px;border-radius:20px;border:1px solid var(--border)}
.stats{display:flex;gap:8px;margin-left:auto;flex-wrap:wrap}
.stat{font-size:11px;padding:3px 8px;border-radius:12px;background:var(--bg3);border:1px solid var(--border);color:var(--muted);display:flex;align-items:center;gap:5px}
.dot{width:7px;height:7px;border-radius:50%}
main{max-width:900px;margin:0 auto;padding:24px 20px}
h2{font-size:14px;color:var(--muted);margin-bottom:16px;font-weight:600}
.card{background:var(--bg2);border:1px solid var(--border);border-radius:10px;padding:16px;margin-bottom:12px;cursor:pointer;transition:background .15s}
.card:hover{background:var(--bg3)}
.card-top{display:flex;align-items:center;gap:10px}
.card .status-dot{width:9px;height:9px;border-radius:50%;flex-shrink:0}
.card .project{font-size:11px;color:var(--muted);background:var(--bg3);padding:2px 6px;border-radius:4px}
.card .title{font-size:14px;font-weight:600;flex:1;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.card .time{font-size:11px;color:var(--muted);flex-shrink:0}
.card .preview{font-size:12px;color:var(--muted);margin-top:8px;white-space:nowrap;overflow:hidden;text-overflow:ellipsis}
.detail{background:var(--bg2);border:1px solid var(--border);border-radius:10px;padding:16px;margin-top:16px}
.detail h3{font-size:15px;margin-bottom:12px}
.msg{padding:8px 12px;border-radius:8px;margin-bottom:8px;font-size:13px;line-height:1.5}
.msg.user{background:var(--bg3);border:1px solid var(--border)}
.msg.assistant{background:var(--bg2);border:1px solid var(--border)}
.msg.system{background:rgba(239,68,68,.1);border:1px solid var(--red);color:var(--red);font-family:Consolas,monospace;font-size:12px}
.msg .role{font-size:11px;color:var(--muted);margin-bottom:2px}
.empty{text-align:center;color:var(--muted);padding:60px 0}
</style>
</head>
<body>
<header>
  <span class="logo">CCBuddy</span>
  <span class="badge">Claude Code 会话管理器</span>
  <div class="stats" id="stats"></div>
</header>
<main>
  <h2>会话事件流</h2>
  <div id="list"></div>
  <div id="detail"></div>
</main>
<script>
const COLORS={running:'var(--green)',waiting_confirmation:'var(--orange)',waiting_input:'var(--blue)',error:'var(--red)',completed:'var(--gray)',idle:'var(--muted)'};
const LABELS={running:'运行中',waiting_confirmation:'需确认',waiting_input:'等待输入',error:'异常',completed:'已完成',idle:'空闲'};
const PRIORITY={waiting_confirmation:0,error:1,waiting_input:2,running:3,idle:4,completed:5};
let sessions=[];
async function refresh(){
  try{
    const r=await fetch('/api/sessions');
    sessions=await r.json();
    sessions.sort((a,b)=>(PRIORITY[a.status]??10)-(PRIORITY[b.status]??10));
    renderStats();renderList();
  }catch(e){console.error(e)}
}
function renderStats(){
  const c={};sessions.forEach(s=>c[s.status]=(c[s.status]||0)+1);
  const order=['running','waiting_confirmation','waiting_input','error','completed'];
  document.getElementById('stats').innerHTML=order.map(k=>
    `<span class="stat"><span class="dot" style="background:${COLORS[k]}"></span>${LABELS[k]} ${c[k]||0}</span>`
  ).join('');
}
function renderList(){
  const el=document.getElementById('list');
  if(!sessions.length){el.innerHTML='<div class="empty">暂无会话，等待 Claude Code 产生事件</div>';return}
  el.innerHTML=sessions.map(s=>`
    <div class="card" onclick="show('${s.id}')">
      <div class="card-top">
        <span class="status-dot" style="background:${COLORS[s.status]}"></span>
        <span class="project">${s.project}</span>
        <span class="title">${s.title}</span>
        <span class="time">${s.lastActivity}</span>
      </div>
      <div class="preview">${s.preview||''}</div>
    </div>`).join('');
}
function show(id){
  const s=sessions.find(x=>x.id===id);if(!s)return;
  document.getElementById('detail').innerHTML=`
    <div class="detail">
      <h3>${s.title} · ${LABELS[s.status]||s.status}</h3>
      ${s.messages.map(m=>`<div class="msg ${m.type}"><div class="role">${m.role==='user'?'👤 用户':m.role==='assistant'?'🤖 Claude':'⚠️ 系统'} ${m.time||''}</div>${m.content}</div>`).join('')}
    </div>`;
}
refresh();setInterval(refresh,2000);
</script>
</body>
</html>"#;
