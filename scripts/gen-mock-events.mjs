// 生成模拟 hook 日志，用于本地测试 CCBuddy 桌面程序。
// 用法：node scripts/gen-mock-events.mjs
//
// 事件序列对齐官方 hook 语义：
// - 每轮响应以 Stop 收尾（Stop 每轮触发，非会话结束）
// - 交互式会话没有 SessionEnd（仅 claude -p / SDK 一次性会话才有）
// - 错误场景用 PostToolUseFailure 驱动（官方无 is_error 标记的通知）
// - 权限场景用 PermissionRequest → PermissionDenied / PostToolUse 驱动
import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const eventsDir = join(homedir(), ".ccbuddy", "events");
mkdirSync(eventsDir, { recursive: true });

const now = Date.now();
const iso = (offsetMs) => new Date(now - offsetMs).toISOString();

const MIN = 60 * 1000;

function ev(receivedAt, hookEvent, payload) {
  return JSON.stringify({ received_at: receivedAt, hook_event: hookEvent, payload });
}

// 每个会话一个文件（符合"按会话分文件"的设计）
const sessions = [
  {
    id: "sess-001",
    cwd: "D:/work/ecommerce-app",
    lines: [
      // 正常一轮：运行中（工具调用进行时）
      ev(iso(15 * MIN), "SessionStart", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", source: "startup" }),
      ev(iso(14 * MIN), "UserPromptSubmit", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", prompt: "帮我排查支付回调为什么总是 500" }),
      ev(iso(13 * MIN), "PreToolUse", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", tool_name: "Bash", tool_input: { command: "curl -X POST ..." } }),
      ev(iso(2 * MIN), "PreToolUse", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", tool_name: "Read", tool_input: { file_path: "src/payments/callback.ts" } }),
    ],
  },
  {
    id: "sess-002",
    cwd: "D:/work/data-pipeline",
    lines: [
      // 正常一轮：工具完成后等待下一轮输入（Stop 收尾 = completed）
      ev(iso(8 * MIN), "SessionStart", { session_id: "sess-002", cwd: "D:/work/data-pipeline", source: "startup" }),
      ev(iso(7 * MIN), "UserPromptSubmit", { session_id: "sess-002", cwd: "D:/work/data-pipeline", prompt: "这个查询在百万级数据下很慢，帮我优化" }),
      ev(iso(6 * MIN), "PreToolUse", { session_id: "sess-002", cwd: "D:/work/data-pipeline", tool_name: "Bash", tool_input: { command: "psql -c 'EXPLAIN ANALYZE ...'" } }),
      ev(iso(3 * MIN), "PostToolUse", { session_id: "sess-002", cwd: "D:/work/data-pipeline", tool_name: "Bash" }),
      ev(iso(20 * 1000), "Stop", { session_id: "sess-002", cwd: "D:/work/data-pipeline" }),
    ],
  },
  {
    id: "sess-003",
    cwd: "D:/work/blog-site",
    lines: [
      // 错误场景：工具执行失败（PostToolUseFailure 驱动，Stop 不覆盖错误状态）
      ev(iso(30 * MIN), "SessionStart", { session_id: "sess-003", cwd: "D:/work/blog-site", source: "startup" }),
      ev(iso(29 * MIN), "UserPromptSubmit", { session_id: "sess-003", cwd: "D:/work/blog-site", prompt: "帮我写一个 Docker 部署脚本" }),
      ev(iso(28 * MIN), "PreToolUse", { session_id: "sess-003", cwd: "D:/work/blog-site", tool_name: "Bash", tool_input: { command: "docker build -t blog ." } }),
      ev(iso(5 * MIN), "PostToolUseFailure", { session_id: "sess-003", cwd: "D:/work/blog-site", tool_name: "Bash" }),
      ev(iso(4 * MIN), "Stop", { session_id: "sess-003", cwd: "D:/work/blog-site" }),
    ],
  },
  {
    id: "sess-004",
    cwd: "D:/work/ecommerce-app",
    lines: [
      // 权限被拒场景：PermissionRequest → PermissionDenied → 复位运行
      ev(iso(90 * MIN), "SessionStart", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", source: "startup" }),
      ev(iso(89 * MIN), "UserPromptSubmit", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", prompt: "根据 controllers 生成 API 文档并删除旧文档" }),
      ev(iso(88 * MIN), "PreToolUse", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", tool_name: "Bash", tool_input: { command: "rm -rf docs/" } }),
      ev(iso(87 * MIN), "PermissionRequest", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", tool_name: "Bash", reason: "Bash command blocked by settings" }),
      ev(iso(80 * MIN), "PermissionDenied", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", tool_name: "Bash" }),
      ev(iso(75 * MIN), "Stop", { session_id: "sess-004", cwd: "D:/work/ecommerce-app" }),
    ],
  },
  {
    id: "sess-005",
    cwd: "D:/work/mobile-app",
    lines: [
      // 等待用户输入场景：Notification 提示需要补充信息
      ev(iso(40 * MIN), "SessionStart", { session_id: "sess-005", cwd: "D:/work/mobile-app", source: "startup" }),
      ev(iso(39 * MIN), "UserPromptSubmit", { session_id: "sess-005", cwd: "D:/work/mobile-app", prompt: "用户反馈登录状态偶尔会丢失，帮我看看" }),
      ev(iso(38 * MIN), "PreToolUse", { session_id: "sess-005", cwd: "D:/work/mobile-app", tool_name: "Grep", tool_input: { pattern: "refreshToken" } }),
      // 注意保持 last_activity 在 10 分钟时效窗内，否则状态机会按时效降级为 idle
      ev(iso(5 * MIN), "Notification", { session_id: "sess-005", cwd: "D:/work/mobile-app", message: "请提供设备日志，这样我可以进一步分析" }),
    ],
  },
  {
    id: "sess-006",
    cwd: "D:/work/data-pipeline",
    lines: [
      // 权限等待中场景：PermissionRequest 后等待用户批准
      ev(iso(70 * MIN), "SessionStart", { session_id: "sess-006", cwd: "D:/work/data-pipeline", source: "startup" }),
      ev(iso(69 * MIN), "UserPromptSubmit", { session_id: "sess-006", cwd: "D:/work/data-pipeline", prompt: "把清洗脚本重构成模块化结构并直接提交" }),
      ev(iso(68 * MIN), "PreToolUse", { session_id: "sess-006", cwd: "D:/work/data-pipeline", tool_name: "Edit", tool_input: { file_path: "clean.py" } }),
      // 注意保持 last_activity 在 10 分钟时效窗内，否则状态机会按时效降级为 idle
      ev(iso(3 * MIN), "PermissionRequest", { session_id: "sess-006", cwd: "D:/work/data-pipeline", tool_name: "Edit", reason: "File outside allowed directories" }),
    ],
  },
];

for (const s of sessions) {
  const filename = `event-${s.id}.jsonl`;
  writeFileSync(join(eventsDir, filename), s.lines.join("\n") + "\n", "utf8");
  console.log(`已生成 ${filename}（${s.lines.length} 行）`);
}

console.log(`\n日志目录：${eventsDir}`);
console.log(`共 ${sessions.length} 个会话。现在运行 npm run tauri dev 即可看到效果。`);
