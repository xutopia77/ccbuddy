/**
 * 轻量 i18n：zh / en 双语字典 + 全局 locale 状态。
 *
 * 机制（对齐设计稿）：界面文案全部走 t(key)；transcript 正文（用户消息/
 * 思考/工具输出）是数据，不随语言切换；Naive UI 组件语言经
 * n-config-provider :locale 联动。t 为普通函数，在模板/计算属性中调用时
 * 读取 locale ref，语言切换自然触发重算。
 *
 * 语言偏好持久化到 localStorage（"ccbuddy-locale"），启动时读取（try/catch）。
 */
import { ref, watch } from "vue";

export type Locale = "zh" | "en";

const STORAGE_KEY = "ccbuddy-locale";

function loadLocale(): Locale {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "zh" || v === "en") return v;
  } catch {
    /* localStorage 不可用（隐私模式等）时用默认值 */
  }
  return "zh";
}

/** 全局语言状态（单一 ref，所有组件共享） */
export const locale = ref<Locale>(loadLocale());

watch(locale, (v) => {
  try {
    localStorage.setItem(STORAGE_KEY, v);
  } catch {
    /* 持久化失败不影响当次会话 */
  }
});

export function setLocale(l: Locale): void {
  locale.value = l;
}

// ---- 双语字典（zh/en 键集完全一致，satisfies 保证） ----
const I18N = {
  // 顶栏
  navUsage: { zh: "用量", en: "Usage" },
  navEvents: { zh: "事件流", en: "Events" },
  navHistory: { zh: "历史会话", en: "History" },
  navSettings: { zh: "设置", en: "Settings" },
  navAria: { zh: "主导航", en: "Main navigation" },
  langAria: { zh: "界面语言", en: "Interface language" },
  aliveOn: { zh: "运行中", en: "Running" },
  aliveOff: { zh: "已断开", en: "Disconnected" },
  aliveOnTitle: { zh: "系统运行正常：后端监听事件目录，有变化自动推送", en: "System healthy: backend watches the events dir and pushes on change" },
  aliveOffTitle: { zh: "与后端推送通道失去连接：检查后端服务是否运行（恢复后将自动重连）", en: "Lost connection to the backend push channel: check that the backend service is running (auto-reconnects on recovery)" },
  pushClockPre: { zh: "最近推送", en: "last push" },

  // App 空态
  eventsEmptyText: { zh: "选择一个会话查看事件流", en: "Pick a session to view its event stream" },
  historyEmptyText: { zh: "选择一个历史会话查看聊天记录", en: "Pick a history session to view the chat" },
  detailEmptyText: { zh: "选择一个会话查看详情", en: "Pick a session to view details" },

  // 列表（EventList / SessionList）
  eventListTitle: { zh: "会话事件流", en: "Event stream" },
  historyListTitle: { zh: "历史会话", en: "History sessions" },
  sessionCountUnit: { zh: "个会话", en: "sessions" },
  unreadTitle: { zh: "有未读事件", en: "Unread events" },
  eventsListEmpty: { zh: "暂无会话，等待 Claude Code 产生事件", en: "No sessions yet — waiting for Claude Code events" },
  retentionPre: { zh: "Claude Code 保留期：", en: "Claude Code retention: " },
  retentionDaysUnit: { zh: " 天，更早的会话已被自动清理", en: " days — older sessions are auto-cleaned" },

  // 历史会话手动刷新（设计稿 .refresh-btn：历史会话不自动刷新，按钮手动重载）
  refreshLabel: { zh: "刷新", en: "Refresh" },
  refreshDone: { zh: "已刷新历史会话", en: "History sessions refreshed" },
  // 事件流条目第二行的消息数量前缀（设计稿 t.msgCount：弱化靠右）
  msgCountPre: { zh: "消息", en: "messages" },

  // 详情（SessionDetail）
  lastActivityLabel: { zh: "最后活动", en: "Last activity" },
  copyBtn: { zh: "复制", en: "Copy" },
  copiedBtn: { zh: "已复制", en: "Copied" },
  actUser: { zh: "用户输入", en: "User input" },
  actClaude: { zh: "Claude", en: "Claude" },
  actThink: { zh: "思考", en: "Thinking" },
  actTool: { zh: "调用工具", en: "Tool call" },
  actToolDone: { zh: "工具完成", en: "Tool done" },
  actToolFail: { zh: "工具失败", en: "Tool failed" },
  actSys: { zh: "系统消息", en: "System" },
  streamTitle: { zh: "事件流时间线", en: "Event timeline" },
  streamCountUnit: { zh: "条", en: "events" },
  streamHint: { zh: "点击行展开详情", en: "click a row to expand details" },
  // 等待横幅（事件流顶部：Claude 在等用户拍板 / 输入）
  waitTitleConfirm: { zh: "Claude 正在等待你的确认", en: "Claude is waiting for your confirmation" },
  waitTitleInput: { zh: "Claude 正在等待你的输入", en: "Claude is waiting for your input" },
  waitGoBtn: { zh: "复制恢复命令", en: "Copy resume command" },
  waitGoHint: { zh: "面板只监控不控制，请回终端拍板：", en: "Monitor-only panel — approve in your terminal: " },
  subagentHeader: { zh: "子代理会话", en: "Subagent sessions" },
  subagentNoDesc: { zh: "(无描述)", en: "(no description)" },
  userInputsHeader: { zh: "用户输入", en: "User inputs" },

  // 状态徽章六态
  statusRunning: { zh: "运行中", en: "Running" },
  statusConfirm: { zh: "需确认", en: "Needs confirmation" },
  statusInput: { zh: "等待输入", en: "Waiting for input" },
  statusError: { zh: "异常", en: "Error" },
  statusDone: { zh: "已完成", en: "Completed" },
  statusIdle: { zh: "空闲", en: "Idle" },

  // 相对时间
  relNow: { zh: "刚刚", en: "just now" },
  relMin: { zh: "分钟前", en: "m ago" },
  relHour: { zh: "小时前", en: "h ago" },
  relDay: { zh: "天前", en: "d ago" },

  // 用量面板
  fmtLabel: { zh: "数值显示", en: "Number format" },
  fmtRaw: { zh: "原始", en: "Raw" },
  refreshSecsLabel: { zh: "更新周期(秒)", en: "Refresh (sec)" },
  statRequests: { zh: "请求数", en: "Requests" },
  statInput: { zh: "输入 Token", en: "Input Tokens" },
  statOutput: { zh: "输出 Token", en: "Output Tokens" },
  statCacheRead: { zh: "缓存命中", en: "Cache Hits" },
  statTotal: { zh: "Token 总量", en: "Total Tokens" },
  statCost: { zh: "总成本", en: "Total cost" },
  subagentPrefix: { zh: "[子代理]", en: "[subagent]" },
  filterSession: { zh: "会话", en: "Session" },
  filterModel: { zh: "模型", en: "Model" },
  allSessions: { zh: "全部会话", en: "All sessions" },
  allModels: { zh: "全部模型", en: "All models" },
  filterCountPre: { zh: "共", en: "" },
  filterCountPost: { zh: "次请求", en: "requests" },
  loadingText: { zh: "加载中…", en: "Loading…" },
  colTime: { zh: "时间", en: "Time" },
  colSession: { zh: "会话", en: "Session" },
  colModel: { zh: "模型", en: "Model" },
  colInput: { zh: "输入", en: "Input" },
  colCacheRead: { zh: "缓存读", en: "Cache R" },
  colCacheWrite: { zh: "缓存写", en: "Cache W" },
  colOutput: { zh: "输出", en: "Output" },
  colTier: { zh: "档位", en: "Tier" },
  tableLoading: { zh: "正在加载…", en: "Loading…" },
  usageEmpty: { zh: "暂无用量数据", en: "No usage data" },

  // 用量趋势图
  trendTitle: { zh: "token 用量趋势", en: "Token usage trend" },
  trendSubMin: { zh: "按 5 分钟聚合 · 输入含缓存读 / 缓存写", en: "5-minute buckets · input incl. cache reads / writes" },
  trendSubDay: { zh: "按天聚合 · 输入含缓存读 / 缓存写", en: "Daily buckets · input incl. cache reads / writes" },
  legendInput: { zh: "输入", en: "Input" },
  legendOutput: { zh: "输出", en: "Output" },
  legendTotal: { zh: "总量", en: "Total" },
  legendCacheHit: { zh: "缓存命中", en: "Cache hits" },
  legendCacheRead: { zh: "缓存读", en: "Cache read" },
  legendCacheWrite: { zh: "缓存写", en: "Cache write" },
  chartEmpty: { zh: "该时间范围暂无数据", en: "No data in this range" },

  // 用量构成环形图
  donutTitle: { zh: "Token 构成", en: "Token composition" },
  donutSub: { zh: "输入 / 输出 / 缓存命中 占总量比例", en: "Input / output / cache-hit share of total" },
  donutCenter: { zh: "Token 总量", en: "Total Tokens" },
  rangeToday: { zh: "今天", en: "Today" },
  range7d: { zh: "近7天", en: "Last 7 days" },
  range14d: { zh: "近14天", en: "Last 14 days" },
  range30d: { zh: "近30天", en: "Last 30 days" },
  range7dShort: { zh: "近7天", en: "7 days" },
  pickStartHint: { zh: "点选起始日期", en: "Pick a start date" },
  pickEndHint: { zh: "点选结束日期（单日范围点同一天）", en: "Pick an end date (same day for a single-day range)" },
  hoverSlotPost: { zh: "时段", en: " slot" },
  hoverToday: { zh: "（今天）", en: " (today)" },

  // 热力图
  heatTitle: { zh: "活跃热力图", en: "Activity heatmap" },
  heatLess: { zh: "少", en: "Less" },
  heatMore: { zh: "多", en: "More" },

  // 设置面板
  settingsTitle: { zh: "⚙️ CCBuddy 设置", en: "⚙️ CCBuddy Settings" },
  hookCardTitle: { zh: "Hook 配置", en: "Hook config" },
  hookCardSub: { zh: "采集 Claude Code 事件的钩子", en: "Hooks capturing Claude Code events" },
  hookStatusLabel: { zh: "Hook Logger 状态", en: "Hook logger status" },
  installed: { zh: "已安装", en: "Installed" },
  notInstalled: { zh: "未安装", en: "Not installed" },
  hookPath: { zh: "可执行文件路径", en: "Executable path" },
  hookRegister: { zh: "Claude settings.json 注册", en: "Claude settings.json registration" },
  registeredPost: { zh: "个事件已注册", en: "events registered" },
  hookDetail: { zh: "注册明细", en: "Registration details" },
  hookOnTitle: { zh: "已注册", en: "registered" },
  hookOffTitle: { zh: "未注册", en: "not registered" },
  hookBroken: { zh: "状态异常", en: "Status abnormal" },
  hookSource: { zh: "hook 安装来源", en: "Hook install source" },
  hookSourceHint: {
    zh: "安装时优先使用本地 hook（安装包内置 / 便携包同目录 / ~/.ccbuddy/bin），否则使用程序内嵌 hook，无需联网",
    en: "Prefers a local hook when installing (bundled with the installer / beside the portable package / ~/.ccbuddy/bin), otherwise the built-in hook is used — no network required",
  },
  installBtn: { zh: "一键安装 / 更新 Hooks", en: "Install / update hooks" },
  installingText: { zh: "正在安装...", en: "Installing..." },
  uninstallBtn: { zh: "卸载 hook", en: "Uninstall hooks" },
  uninstallConfirm: {
    zh: "将移除 ccbuddy-hook 注册并删除已安装的 hook 文件，你的其他配置不受影响。确认卸载？",
    en: "This removes the ccbuddy-hook registration and deletes installed hook files; other settings are untouched. Uninstall?",
  },
  serverCardTitle: { zh: "服务器", en: "Server" },
  serverCardSub: { zh: "Web 版访问地址", en: "Web access address" },
  serverUrlLabel: { zh: "Web 访问地址", en: "Web address" },
  portLabel: { zh: "端口冲突处理", en: "Port conflict handling" },
  portValue: { zh: "启动时检测，冲突则报错", en: "Detected at startup; errors out on conflict" },
  dirCardTitle: { zh: "数据目录", en: "Data directories" },
  dirCardSub: { zh: "claude_dir 可写 · 其余只读", en: "claude_dir writable · others read-only" },
  dirClaude: { zh: "Claude 数据目录", en: "Claude data dir" },
  dirData: { zh: "软件数据目录", en: "App data dir" },
  claudeDirPh: { zh: "留空使用默认 ~/.claude", en: "Empty for default ~/.claude" },
  saveBtn: { zh: "保存", en: "Save" },
  savedBtn: { zh: "已保存", en: "Saved" },

  // 关于（软件版本 / 仓库 / 官网）
  aboutCardTitle: { zh: "关于", en: "About" },
  aboutCardSub: { zh: "版本与链接", en: "Version and links" },
  aboutVersion: { zh: "软件版本", en: "Version" },
  aboutRepo: { zh: "GitHub 仓库", en: "GitHub repository" },
  aboutSite: { zh: "软件官网", en: "Website" },

  // 登录页（仅 Web 版 / ccbuddy-server；桌面端无登录概念）
  loginTitle: { zh: "登录", en: "Sign in" },
  loginUsername: { zh: "用户名", en: "Username" },
  loginPassword: { zh: "密码", en: "Password" },
  loginBtn: { zh: "登 录", en: "Sign in" },
  loginEmpty: { zh: "请输入密码", en: "Enter your password" },
  loginFailed: { zh: "密码错误，请重试", en: "Wrong password, please try again" },
  loginNetFail: { zh: "无法连接服务，请确认后端已启动", en: "Cannot reach the service — make sure the backend is running" },

  // 设置页 · 访问密码（仅 Web 版有意义的卡片）
  pwdCardTitle: { zh: "访问密码", en: "Access password" },
  pwdCardSub: { zh: "Web 版登录密码 · 修改后需重新登录", en: "Web sign-in password · re-login required after change" },
  pwdOld: { zh: "当前密码", en: "Current password" },
  pwdNew: { zh: "新密码", en: "New password" },
  pwdConfirm: { zh: "确认新密码", en: "Confirm new password" },
  pwdPh: { zh: "至少 6 位", en: "At least 6 characters" },
  pwdChangeBtn: { zh: "修改密码", en: "Change password" },
  pwdMismatch: { zh: "两次输入的新密码不一致", en: "The new passwords do not match" },
  pwdChanged: { zh: "密码已修改，请重新登录", en: "Password changed — please sign in again" },
  logoutBtn: { zh: "退出登录", en: "Sign out" },

  // 设置 toast
  toastInstallDone: { zh: "Hook 安装完成", en: "Hooks installed" },
  toastUninstallDone: { zh: "Hook 已卸载", en: "Hooks uninstalled" },
  toastInstallFail: { zh: "安装失败", en: "Install failed" },
  toastUninstallFail: { zh: "卸载失败", en: "Uninstall failed" },
  toastSaveFail: { zh: "保存失败", en: "Save failed" },
} satisfies Record<string, { zh: string; en: string }>;

export type I18nKey = keyof typeof I18N;

/** 取当前语言文案（模板/计算属性中调用，locale 变化自然触发重算） */
export function t(key: I18nKey): string {
  return I18N[key][locale.value];
}

// ---- 数组类文案（月标签 / 周几），随 locale 切换 ----
const MONTHS_EN = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];

/** 热力图月标签（1月..12月 / Jan..Dec） */
export function heatMonths(): string[] {
  return locale.value === "zh"
    ? ["1月", "2月", "3月", "4月", "5月", "6月", "7月", "8月", "9月", "10月", "11月", "12月"]
    : MONTHS_EN;
}

/** 热力图周几行（只标一/三/五 ↔ Mon/Wed/Fri，其余空占位） */
export function heatWeekdays(): string[] {
  return locale.value === "zh" ? ["", "一", "", "三", "", "五", ""] : ["", "Mon", "", "Wed", "", "Fri", ""];
}

/** 热力图副标题：`近一年 · 累计 N 次请求 · 最长连续 M 天活跃`（reqText 已格式化）。 */
export function heatSubText(reqText: string, best: number): string {
  return locale.value === "zh"
    ? `近一年 · 累计 ${reqText} 次请求 · 最长连续 ${best} 天活跃`
    : `Past year · ${reqText} requests · longest streak ${best} days`;
}

/** 月历星期行（日一二三四五六 / Su..Sa） */
export function calWeekdays(): string[] {
  return locale.value === "zh"
    ? ["日", "一", "二", "三", "四", "五", "六"]
    : ["Su", "Mo", "Tu", "We", "Th", "Fr", "Sa"];
}

/** 月历标题（2026年3月 / Mar 2026） */
export function calTitle(d: Date): string {
  return locale.value === "zh"
    ? `${d.getFullYear()}年${d.getMonth() + 1}月`
    : `${MONTHS_EN[d.getMonth()]} ${d.getFullYear()}`;
}
