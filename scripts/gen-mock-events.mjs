// 生成模拟 hook 日志，用于本地测试 CCBuddy 桌面程序。
// 用法：node scripts/gen-mock-events.mjs
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
      ev(iso(15 * MIN), "SessionStart", { session_id: "sess-001", cwd: "D:/work/ecommerce-app" }),
      ev(iso(14 * MIN), "UserPromptSubmit", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", prompt: "帮我排查支付回调为什么总是 500" }),
      ev(iso(13 * MIN), "AssistantMessage", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", message: "我来检查一下支付服务的日志和回调处理逻辑。" }),
      ev(iso(2 * MIN), "PreToolUse", { session_id: "sess-001", cwd: "D:/work/ecommerce-app", tool_name: "Bash", tool_input: { command: "curl -X POST ..." } }),
    ],
  },
  {
    id: "sess-002",
    cwd: "D:/work/data-pipeline",
    lines: [
      ev(iso(8 * MIN), "SessionStart", { session_id: "sess-002", cwd: "D:/work/data-pipeline" }),
      ev(iso(7 * MIN), "UserPromptSubmit", { session_id: "sess-002", cwd: "D:/work/data-pipeline", prompt: "这个查询在百万级数据下很慢，帮我优化" }),
      ev(iso(6 * MIN), "AssistantMessage", { session_id: "sess-002", cwd: "D:/work/data-pipeline", message: "我先查看表结构和现有索引，然后分析执行计划。" }),
      ev(iso(20 * 1000), "PostToolUse", { session_id: "sess-002", cwd: "D:/work/data-pipeline", tool_name: "Bash" }),
    ],
  },
  {
    id: "sess-003",
    cwd: "D:/work/blog-site",
    lines: [
      ev(iso(30 * MIN), "SessionStart", { session_id: "sess-003", cwd: "D:/work/blog-site" }),
      ev(iso(29 * MIN), "UserPromptSubmit", { session_id: "sess-003", cwd: "D:/work/blog-site", prompt: "帮我写一个 Docker 部署脚本" }),
      ev(iso(28 * MIN), "AssistantMessage", { session_id: "sess-003", cwd: "D:/work/blog-site", message: "好的，我来编写 Dockerfile 和 docker-compose.yml。" }),
      ev(iso(5 * MIN), "Notification", { session_id: "sess-003", cwd: "D:/work/blog-site", message: "Error: Connection refused when pushing image to registry", is_error: true }),
    ],
  },
  {
    id: "sess-004",
    cwd: "D:/work/ecommerce-app",
    lines: [
      ev(iso(90 * MIN), "SessionStart", { session_id: "sess-004", cwd: "D:/work/ecommerce-app" }),
      ev(iso(89 * MIN), "UserPromptSubmit", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", prompt: "根据 controllers 生成 API 文档" }),
      ev(iso(88 * MIN), "AssistantMessage", { session_id: "sess-004", cwd: "D:/work/ecommerce-app", message: "已扫描所有控制器，生成文档完成。" }),
      ev(iso(60 * MIN), "SessionEnd", { session_id: "sess-004", cwd: "D:/work/ecommerce-app" }),
    ],
  },
  {
    id: "sess-005",
    cwd: "D:/work/mobile-app",
    lines: [
      ev(iso(40 * MIN), "SessionStart", { session_id: "sess-005", cwd: "D:/work/mobile-app" }),
      ev(iso(39 * MIN), "UserPromptSubmit", { session_id: "sess-005", cwd: "D:/work/mobile-app", prompt: "用户反馈登录状态偶尔会丢失，帮我看看" }),
      ev(iso(38 * MIN), "AssistantMessage", { session_id: "sess-005", cwd: "D:/work/mobile-app", message: "这可能是 token 刷新机制导致的。" }),
      ev(iso(10 * MIN), "Notification", { session_id: "sess-005", cwd: "D:/work/mobile-app", message: "请提供设备日志，这样我可以进一步分析" }),
    ],
  },
  {
    id: "sess-006",
    cwd: "D:/work/data-pipeline",
    lines: [
      ev(iso(70 * MIN), "SessionStart", { session_id: "sess-006", cwd: "D:/work/data-pipeline" }),
      ev(iso(69 * MIN), "UserPromptSubmit", { session_id: "sess-006", cwd: "D:/work/data-pipeline", prompt: "先看看现有清洗脚本的结构" }),
      ev(iso(30 * MIN), "AssistantMessage", { session_id: "sess-006", cwd: "D:/work/data-pipeline", message: "我已经分析了脚本，发现重复代码较多。你想让我开始重构吗？" }),
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
