# CCBuddy

Claude Code 会话监控面板：实时掌握多个 Claude Code 会话的运行状态、事件流与历史记录。

- **事件流**：通过 Hook 实时采集会话事件（工具调用、用户输入、通知等），状态机推断每个会话的最新状态（运行中 / 等待确认 / 等待输入 / 出错 / 已完成）
- **历史会话**：直接读取 Claude Code 原生会话记录（`~/.claude/projects/`），以聊天记录形式回放完整对话
- **双形态发布**：Tauri2 桌面端（Windows / Linux / macOS）+ 无头服务端（`ccbuddy-server`，无桌面环境的 Linux 服务器，浏览器访问）

## 工作原理

```
Claude Code ──hook──> ccbuddy-hook ──append──> ~/.ccbuddy/events/event-<session-id>.jsonl
                                                                    │
CCBuddy（桌面端 / 服务端） <──增量解析 + 状态机── 读文件（按 mtime 缓存）──┘
                                        │
Claude Code 原生记录 ~/.claude/projects/ ┘（历史会话视图，只读）
```

1. **hook 采集**：`ccbuddy-hook` 注册为 Claude Code 的 Hooks（settings.json），每次事件触发时把 stdin 的 JSON 包装写入按会话分文件的 JSONL 日志
2. **事件解析**：主程序读取 `~/.ccbuddy/events/`，按文件 mtime 增量解析（未变化的文件直接复用缓存），每个会话只保留最新 50 条事件
3. **状态机**：事件序列（SessionStart / UserPromptSubmit / PreToolUse / PostToolUse / Notification / Stop / SessionEnd）驱动会话状态推断，等待确认与出错的状态会标记未读

## 架构

前后端通过统一 RPC 协议通信（`{time, cmd, data}` / `{time, cmd, code, status, data}`），业务代码不接触 Tauri / HTTP 细节：

```
前端（Vue3 + Naive UI）
  业务组件 ──> api.ts（业务函数）──> ipc.ts（RPC 客户端：Tauri invoke / HTTP POST 自适应）

后端（Rust）
  适配层 gui 模块（Tauri）/ server.rs（axum）──> core::dispatch（命令路由，纯 Rust）
    └── state.rs（事件解析与状态机） / event.rs（事件模型） / config.rs（用户配置）
```

- **命令清单**：`get_events`、`get_sessions`、`get_event_detail`、`get_session_detail`、`get_config`、`set_config`、`get_hook_status`、`install_hooks`
- 新增命令只需改 `core.rs` 路由表与前端 `api.ts`，两个适配层零改动

## 快速开始

### 桌面端

```bash
npm install
npm run tauri dev    # 开发
npm run tauri build  # 打包
```

首次使用进入设置页点击"一键安装 / 更新 Hooks"：hook 优先使用本地文件（安装包内置 / 便携包同目录 / `~/.ccbuddy/bin`），本地没有则自动从 GitHub Release 下载；离线环境按提示手动下载放置即可。

### 服务端（无桌面环境）

```bash
cargo build --no-default-features --bin ccbuddy-server
./ccbuddy-server 0.0.0.0:8787   # 浏览器访问 http://<host>:8787
```

前端静态资源在构建时嵌入二进制（include_dir），单文件部署。

## 数据目录

| 路径 | 用途 |
|------|------|
| `~/.ccbuddy/events/` | 事件流日志（hook 写入，`event-<session-id>.jsonl`） |
| `~/.ccbuddy/config.json` | 用户配置（Claude 目录、GitHub 仓库地址等） |
| `~/.ccbuddy/bin/` | 手动下载的 hook 放置目录 |
| `~/.claude/projects/` | Claude Code 原生会话记录（只读） |

## 开发

- Rust：`cargo test --manifest-path src-tauri/Cargo.toml --lib`
- 前端：`npx vue-tsc --noEmit`
- 样式：design token 统一定义于 `src/styles/tokens.css`

## License

MIT
