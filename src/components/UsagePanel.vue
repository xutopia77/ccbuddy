<template>
  <div class="usage-panel">
    <!-- 数值显示模式选择 + 用量更新周期（后端 watcher 间隔，变化即推送） -->
    <div class="fmt-bar">
      <span class="filter-label">{{ t("fmtLabel") }}</span>
      <n-select
        v-model:value="fmtMode"
        size="small"
        class="fmt-select"
        :options="fmtOptions()"
      />
      <span class="filter-label">{{ t("refreshSecsLabel") }}</span>
      <n-input-number
        v-model:value="refreshSecs"
        size="small"
        class="fmt-select"
        :min="5"
        :max="3600"
        :step="5"
        @update:value="onRefreshSecsCommit"
      />
    </div>

    <!-- 页头统计卡：彩色图标 + 名称 + 数值；卡内色相即该指标在趋势图/环形图中的色相。
         右上角迷你曲线是固定点位的装饰图形（不接真实数据，仅作视觉点缀）。 -->
    <div class="stat-cards">
      <div
        v-for="c in statCards"
        :key="c.key"
        class="stat-card"
        :style="{ color: c.color }"
      >
        <svg class="stat-spark" viewBox="0 0 64 22" aria-hidden="true">
          <polyline :points="c.spark" />
        </svg>
        <div class="stat-head">
          <span class="stat-ico">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path v-for="(d, i) in c.icon" :key="i" :d="d" />
            </svg>
          </span>
          <span class="stat-name">{{ c.label }}</span>
        </div>
        <div class="stat-value">{{ fmtNum(c.value) }}</div>
      </div>
      <!-- L-C9：总成本卡（记录带成本字段才显示，无成本数据隐藏） -->
      <div v-if="stats.cost !== undefined" class="stat-card">
        <div class="stat-head">
          <span class="stat-ico">
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path v-for="(d, i) in ICON_COST" :key="i" :d="d" />
            </svg>
          </span>
          <span class="stat-name">{{ t("statCost") }}</span>
        </div>
        <div class="stat-value">{{ fmtCost(stats.cost) }}</div>
      </div>
    </div>

    <!-- 用量趋势折线图 + Token 构成环形图（同一行）。
         时间范围选择器在趋势图头部，状态提升到本组件：统计卡、趋势图、
         环形图三者共用同一份口径（当前范围 ∩ 会话/模型筛选）。 -->
    <div class="chart-row">
      <UsageTrendChart
        v-model="rangeSelection"
        :records="records"
        :fmt-mode="fmtMode"
      />
      <UsageDonutChart
        :total="stats.total"
        :input="stats.input"
        :output="stats.output"
        :cache-read="stats.cacheRead"
        :fmt-mode="fmtMode"
      />
    </div>

    <!-- 活跃热力图（GitHub 贡献图式：列=周，行=周日..周六；
         固定近一年，不随时间范围/筛选联动；口径 = 输入+缓存读+缓存写+输出） -->
    <section class="heat-card">
      <div class="heat-head">
        <div class="heat-title">{{ t("heatTitle") }}</div>
        <div class="heat-sub">{{ heatSubLabel }}</div>
      </div>
      <div class="heat-scroll">
        <div class="heat-inner">
          <div class="heat-months">
            <span v-for="mo in heatMonthLabels" :key="mo.key" :style="{ left: mo.left }">{{ mo.label }}</span>
          </div>
          <div class="heat-grid-row">
            <div class="heat-days">
              <span v-for="(w, wi) in heatWeekdays()" :key="wi">{{ w }}</span>
            </div>
            <div v-for="wk in heatWeeks" :key="wk.key" class="heat-week">
              <div
                v-for="cell in wk.cells"
                :key="cell.key"
                class="heat-cell"
                :class="[cell.blank ? 'blank' : 'lv' + heatLevel(cell.total!)]"
                :style="cell.blank ? undefined : { background: `var(--heat-${heatLevel(cell.total!)})` }"
                @mouseenter="heatEnter($event, cell)"
                @mouseleave="heatLeave"
              ></div>
            </div>
          </div>
        </div>
      </div>
      <div class="heat-legend">
        <span>{{ t("heatLess") }}</span>
        <span v-for="lv in [0, 1, 2, 3, 4]" :key="lv" class="lg-cell" :style="{ background: `var(--heat-${lv})` }"></span>
        <span>{{ t("heatMore") }}</span>
      </div>
      <div v-if="heatTip" class="heat-tip" :style="{ left: heatTip.x + 'px', top: heatTip.y + 'px' }">
        <span class="ht-date">{{ heatTip.date }}</span>
        <span class="ht-row"><span>{{ t("colInput") }}</span><b>{{ fmtNum(heatTip.input) }}</b></span>
        <span class="ht-row"><span>{{ t("colCacheRead") }}</span><b>{{ fmtNum(heatTip.cacheRead) }}</b></span>
        <span class="ht-row"><span>{{ t("colCacheWrite") }}</span><b>{{ fmtNum(heatTip.cacheWrite) }}</b></span>
        <span class="ht-row"><span>{{ t("colOutput") }}</span><b>{{ fmtNum(heatTip.output) }}</b></span>
        <span class="ht-row total"><span>{{ t("legendTotal") }}</span><b>{{ fmtNum(heatTip.total) }}</b></span>
      </div>
    </section>

    <!-- 筛选栏 -->
    <div class="filter-bar">
      <span class="filter-label">{{ t("filterSession") }}</span>
      <n-select
        v-model:value="filterSession"
        size="small"
        class="filter-select"
        :options="sessionOptions"
        @update:value="onFilterChange"
      />
      <span class="filter-label">{{ t("filterModel") }}</span>
      <n-select
        v-model:value="filterModel"
        size="small"
        class="filter-select"
        :options="modelOptions"
        @update:value="onFilterChange"
      />
      <span class="filter-count">{{ t("filterCountPre") }} {{ fmtNum(totalFiltered) }} {{ t("filterCountPost") }}</span>
      <span v-if="loading" class="filter-count">{{ t("loadingText") }}</span>
    </div>

    <!-- 用量表格（全宽单栏） -->
    <div class="table-wrap">
      <table class="usage-table">
        <thead>
          <tr>
            <th class="sortable" @click="toggleSort('timestamp')">
              {{ t("colTime") }}<span class="arrow">{{ sortKey === 'timestamp' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="sortable" @click="toggleSort('session')">
              {{ t("colSession") }}<span class="arrow">{{ sortKey === 'session' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="sortable" @click="toggleSort('model')">
              {{ t("colModel") }}<span class="arrow">{{ sortKey === 'model' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="num sortable" @click="toggleSort('inputTokens')">
              {{ t("colInput") }}<span class="arrow">{{ sortKey === 'inputTokens' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="num sortable" @click="toggleSort('cacheReadTokens')">
              {{ t("colCacheRead") }}<span class="arrow">{{ sortKey === 'cacheReadTokens' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="num sortable" @click="toggleSort('cacheCreationTokens')">
              {{ t("colCacheWrite") }}<span class="arrow">{{ sortKey === 'cacheCreationTokens' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="num sortable" @click="toggleSort('outputTokens')">
              {{ t("colOutput") }}<span class="arrow">{{ sortKey === 'outputTokens' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
            <th class="sortable" @click="toggleSort('serviceTier')">
              {{ t("colTier") }}<span class="arrow">{{ sortKey === 'serviceTier' ? (sortDesc ? '↓' : '↑') : '' }}</span>
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-if="pageRecords.length === 0">
            <td colspan="8" class="empty-cell">{{ loading ? t("tableLoading") : t("usageEmpty") }}</td>
          </tr>
          <tr v-for="(r, i) in pageRecords" :key="r.sessionId + r.timestamp + i">
            <td class="cell-time">{{ fmtDateTime(r.timestamp) }}</td>
            <td>
              <n-tooltip placement="top-start" :show-arrow="false">
                <template #trigger>
                  <span class="cell-session">{{ sessionTitle(r) }}</span>
                </template>
                {{ sessionTitles.get(r.sessionId) || r.sessionId }}
              </n-tooltip>
            </td>
            <td class="cell-model">{{ r.model || "-" }}</td>
            <td class="num">{{ fmtNum(r.inputTokens) }}</td>
            <td class="num">{{ fmtNum(r.cacheReadTokens) }}</td>
            <td class="num">{{ fmtNum(r.cacheCreationTokens) }}</td>
            <td class="num">{{ fmtNum(r.outputTokens) }}</td>
            <td class="cell-tier">{{ r.serviceTier || "-" }}</td>
          </tr>
        </tbody>
      </table>
    </div>

    <!-- 分页（底部留白收尾，滚动时不贴底） -->
    <div class="pager">
      <n-pagination
        :page="page"
        :page-count="totalPages"
        size="small"
        @update:page="page = $event"
      />
    </div>
    <div class="panel-tail" aria-hidden="true"></div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount, watch } from "vue";
import type { SelectOption } from "naive-ui";
import { getUsage, getSessions, getConfig, setConfig } from "../api";
import { onEvent } from "../ipc";
import type { UsageRecord } from "../types";
import { fmtDateTime } from "../types";
import { fmtOptions, fmtValue, type FmtMode } from "../usageFmt";
import { t, heatMonths, heatWeekdays, heatSubText } from "../i18n";
import UsageTrendChart, { type RangeSelection } from "./UsageTrendChart.vue";
import UsageDonutChart from "./UsageDonutChart.vue";

// ---- 数据 ----
const records = ref<UsageRecord[]>([]);
const loading = ref(false);
// 会话标题映射：sessionId → 标题（表格展示会话名而非裸 id）
const sessionTitles = ref<Map<string, string>>(new Map());

// ---- 数值显示模式（整个用量界面共用：卡片 / 图表 / 表格）----
// 默认 K/M/G；选择持久化到 localStorage（浏览器存储，读写均 try/catch 降级）
const FMT_STORE_KEY = "ccbuddy:usage-fmt";
function loadFmtMode(): FmtMode {
  try {
    const v = localStorage.getItem(FMT_STORE_KEY);
    if (v === "raw" || v === "si" || v === "cn") return v;
  } catch {
    /* localStorage 不可用时用默认 */
  }
  return "si";
}
const fmtMode = ref<FmtMode>(loadFmtMode());
watch(fmtMode, (v) => {
  try {
    localStorage.setItem(FMT_STORE_KEY, v);
  } catch {
    /* 持久化失败仅本次会话内生效 */
  }
});

// ---- 时间范围（图表选择器驱动，作用于统计卡与图表；表格/请求计数不受影响）----
// 状态提升到本组件：图表通过 v-model 消费/变更。默认近30天，选择持久化到 localStorage。
const RANGE_STORE_KEY = "ccbuddy:usage-range";
function loadRangeSelection(): RangeSelection {
  try {
    const raw = localStorage.getItem(RANGE_STORE_KEY);
    if (raw) {
      const v = JSON.parse(raw) as RangeSelection;
      if (v && typeof v.key === "string") {
        return { key: v.key as RangeSelection["key"], custom: v.custom ?? null };
      }
    }
  } catch {
    /* 解析失败用默认 */
  }
  return { key: "30d", custom: null };
}
const rangeSelection = ref<RangeSelection>(loadRangeSelection());
watch(rangeSelection, (v) => {
  try {
    localStorage.setItem(RANGE_STORE_KEY, JSON.stringify(v));
  } catch {
    /* 持久化失败仅本次会话内生效 */
  }
});

// ---- 更新周期（秒）：后端 watcher 按此间隔检查 transcript 并推送 usage_changed ----
const refreshSecs = ref<number | null>(30);

// ---- 筛选 ----
const filterSession = ref("");
const filterModel = ref("");

// ---- 排序 ----
// sortKey 为 null 时按默认（时间倒序）展示；再次点击同列切换升/降序
type SortKey =
  | "timestamp" | "session" | "model"
  | "inputTokens" | "cacheReadTokens" | "cacheCreationTokens" | "outputTokens" | "serviceTier";
const sortKey = ref<SortKey | null>(null);
const sortDesc = ref(true);

// ---- 分页 ----
const PAGE_SIZE = 50;
const page = ref(1);

// ---- 数据加载 ----
/** 拉取全量用量记录 + 会话标题映射。组件每次挂载（切换到用量 tab）都会触发，
 * 后端做 mtime 增量扫描，活跃会话数据保持新鲜。 */
async function loadUsage() {
  loading.value = true;
  try {
    const usage = await getUsage();
    applyUsage(usage);
  } catch (e) {
    console.error("加载用量数据失败", e);
  } finally {
    loading.value = false;
  }
}

/** 应用记录到界面（初始加载与 usage_changed 推送共用）。 */
function applyUsage(usage: UsageRecord[]) {
  records.value = usage;
  // 筛选可能因新数据越界，回第 1 页
  page.value = 1;
}

/** 拉取会话标题映射（失败不阻塞表格）。 */
async function loadTitles() {
  const sessions = await getSessions().catch(() => [] as Awaited<ReturnType<typeof getSessions>>);
  const titles = new Map<string, string>();
  for (const s of sessions) {
    titles.set(s.id, s.title);
  }
  sessionTitles.value = titles;
}

// ---- 用量推送订阅：后端 watcher 检测到 transcript 变化时主动推送 ----
// （SSE / Tauri event 同一信封协议，与事件流 events_changed 同构）
const unlistenUsage = onEvent<UsageRecord[]>("usage_changed", (usage) => {
  applyUsage(usage);
  void loadTitles();
});

onMounted(async () => {
  // 初始值：配置里的刷新周期（加载一次，后续改动走输入框提交）
  try {
    const cfg = await getConfig();
    refreshSecs.value = cfg.usage_refresh_secs;
  } catch {
    refreshSecs.value = 30;
  }
  await loadUsage();
  void loadTitles();
});
onBeforeUnmount(() => unlistenUsage());

/** 更新周期输入提交：写配置，后端 watcher 下一轮读取生效。 */
async function onRefreshSecsCommit(v: number | null) {
  if (v === null) return;
  try {
    const cfg = await setConfig({ usage_refresh_secs: v });
    refreshSecs.value = cfg.usage_refresh_secs; // 回显后端钳制值（5~3600）
  } catch (e) {
    console.error("更新刷新周期失败", e);
  }
}

// ---- 派生数据 ----
/** 用量中实际出现过的会话（去重），下拉选项（首项"全部会话"对应原空 value）。
 *  H-C2：子代理转录（agent-<id>）没有主会话标题，加「[子代理]」前缀区分。 */
const sessionOptions = computed<SelectOption[]>(() => {
  const ids = new Set<string>();
  for (const r of records.value) {
    ids.add(r.sessionId);
  }
  const out: SelectOption[] = [{ label: t("allSessions"), value: "" }];
  for (const id of ids) {
    const label = sessionTitles.value.get(id)
      || (r_isSubagentId(id) ? `${t("subagentPrefix")} ${id}` : id);
    out.push({ label, value: id });
  }
  return out;
});

/** H-C2：子代理转录的会话 id 形如 agent-<hex>（usage_mgr 扫描文件名派生）。 */
function r_isSubagentId(id: string): boolean {
  return id.startsWith("agent-");
}

/** 用量中实际出现过的模型（去重），下拉选项（首项"全部模型"对应原空 value）。 */
const modelOptions = computed<SelectOption[]>(() => {
  const models = new Set<string>();
  for (const r of records.value) {
    if (r.model) models.add(r.model);
  }
  return [
    { label: t("allModels"), value: "" },
    ...Array.from(models).sort().map((m) => ({ label: m, value: m })),
  ];
});

/** 筛选 + 排序后的记录（分页前的全集；表格/请求计数直接用它，统计卡再叠加时间范围过滤）。 */
const displayRecords = computed(() => {
  let list = records.value;
  if (filterSession.value) {
    list = list.filter((r) => r.sessionId === filterSession.value);
  }
  if (filterModel.value) {
    list = list.filter((r) => r.model === filterModel.value);
  }

  if (sortKey.value === null) {
    // 默认时间倒序（后端已排序，直接复制）
    return [...list];
  }
  const key = sortKey.value;
  const desc = sortDesc.value;
  return [...list].sort((a, b) => {
    let cmp: number;
    if (key === "session") {
      cmp = sessionTitle(a).localeCompare(sessionTitle(b));
    } else if (key === "timestamp" || key === "model" || key === "serviceTier") {
      const av = String(a[key] ?? "");
      const bv = String(b[key] ?? "");
      cmp = av.localeCompare(bv);
    } else {
      cmp = a[key] - b[key];
    }
    return desc ? -cmp : cmp;
  });
});

const totalFiltered = computed(() => displayRecords.value.length);
const totalPages = computed(() => Math.max(1, Math.ceil(totalFiltered.value / PAGE_SIZE)));

/** 当前时间范围的过滤边界（毫秒，本地时区），口径与图表聚合一致：
 *  预设 = [today-(N-1) 00:00, now]；自定义 = 月历起止（起 00:00 ~ 止 23:59:59.999）。
 *  key 为 custom 但无起止对（UI 不可达）时按近30天兜底（图表 dailySeries 同样兜底）。 */
const rangeBounds = computed<[number, number]>(() => {
  const sel = rangeSelection.value;
  const now = new Date();
  if (sel.key === "custom" && sel.custom) return sel.custom;
  const days =
    sel.key === "today" ? 1 : sel.key === "7d" ? 7 : sel.key === "14d" ? 14 : 30;
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1));
  return [start.getTime(), now.getTime()];
});

/** 统计卡记录集：会话/模型筛选 ∩ 时间范围。
 *  时间戳无法解析的记录按"不在范围内"排除（不抛错）。 */
const statsRecords = computed(() => {
  const [s, e] = rangeBounds.value;
  return displayRecords.value.filter((r) => {
    const t = new Date(r.timestamp).getTime();
    return !isNaN(t) && t >= s && t <= e;
  });
});

/** 当前页记录：页码越界（筛选后）时钳回第 1 页由 watcher 处理，这里再兜底。 */
const pageRecords = computed(() => {
  const start = (page.value - 1) * PAGE_SIZE;
  return displayRecords.value.slice(start, start + PAGE_SIZE);
});

/** 页头统计卡（时间范围 ∩ 会话/模型筛选后的记录集，跟随两者联动）。 */
const stats = computed(() => {
  let input = 0;
  let output = 0;
  let cacheRead = 0;
  let cacheWrite = 0;
  let cost = 0;
  let hasCost = false;
  for (const r of statsRecords.value) {
    input += r.inputTokens;
    output += r.outputTokens;
    cacheRead += r.cacheReadTokens;
    cacheWrite += r.cacheCreationTokens;
    // L-C9：成本（新版 transcript 才有；任一记录带 cost 即显示总成本卡）
    if (typeof r.cost === "number") {
      hasCost = true;
      cost += r.cost;
    }
  }
  return {
    requests: statsRecords.value.length,
    input,
    output,
    cacheRead,
    // 总量 = 输入 + 缓存读 + 缓存写 + 输出（所有 token 加在一起的量）
    total: input + output + cacheRead + cacheWrite,
    // L-C9：无成本数据时 undefined（前端隐藏总成本卡）
    cost: hasCost ? cost : undefined,
  };
});

/** L-C9：成本格式化（4 位小数足够美元级成本展示）。 */
function fmtCost(v: number): string {
  return "$" + v.toFixed(4);
}

// ---- 统计卡（图标 + 名称 + 数值）----
// 图标为 24×24 线性图标（path 的 d 数组），配色取自 tokens.css 用量系列 token：
// 与趋势图折线、环形图扇区同源，同一指标在三个位置始终同色。
const ICON_SIGMA = ["M17 4H7l6 8-6 8h10"];
const ICON_PULSE = ["M3 12h4l3-7 4 14 3-7h4"];
const ICON_DOWN = ["M12 3v10", "m8 9 4 4 4-4", "M4 18h16"];
const ICON_UP = ["M12 13V3", "m8 7 4-4 4 4", "M4 18h16"];
const ICON_LAYERS = ["m12 3 9 5-9 5-9-5 9-5z", "m3 13 9 5 9-5"];
const ICON_COST = ["M12 2v20", "M17 6H9.5a3.5 3.5 0 0 0 0 7h5a3.5 3.5 0 0 1 0 7H6"];

// 卡内装饰曲线点位（64×22 viewBox，纯装饰不接真实数据）。
// 直接取自 tmp/ui-example.html 的 sparkline()：同一份种子演示数据最后 30 天的
// 归一化折线，五项各自形态不同，此处固化为常量。
const SPARK_TOTAL =
  "0.0,6.9 2.2,17.5 4.4,19.1 6.6,8.6 8.8,8.6 11.0,17.1 13.2,13.6 15.4,9.6 17.7,8.1 19.9,16.4 22.1,9.7 24.3,3.0 26.5,13.7 28.7,15.7 30.9,9.6 33.1,17.6 35.3,20.0 37.5,11.3 39.7,9.7 41.9,9.8 44.1,12.2 46.3,13.6 48.6,18.2 50.8,14.6 53.0,13.3 55.2,14.4 57.4,8.0 59.6,8.6 61.8,10.9 64.0,13.1";
const SPARK_REQUESTS =
  "0.0,3.0 2.2,20.0 4.4,20.0 6.6,17.2 8.8,17.2 11.0,17.2 13.2,17.2 15.4,8.7 17.7,11.5 19.9,20.0 22.1,11.5 24.3,3.0 26.5,20.0 28.7,17.2 30.9,11.5 33.1,17.2 35.3,20.0 37.5,20.0 39.7,20.0 41.9,17.2 44.1,8.7 46.3,17.2 48.6,20.0 50.8,11.5 53.0,20.0 55.2,20.0 57.4,3.0 59.6,8.7 61.8,17.2 64.0,17.2";
const SPARK_INPUT =
  "0.0,10.5 2.2,18.4 4.4,20.0 6.6,9.3 8.8,6.3 11.0,17.6 13.2,15.9 15.4,12.1 17.7,8.7 19.9,17.2 22.1,11.4 24.3,3.0 26.5,12.0 28.7,14.6 30.9,11.7 33.1,17.8 35.3,19.5 37.5,15.7 39.7,13.6 41.9,3.6 44.1,9.6 46.3,11.6 48.6,17.2 50.8,11.6 53.0,16.9 55.2,17.7 57.4,4.3 59.6,9.9 61.8,8.8 64.0,16.6";
const SPARK_OUTPUT =
  "0.0,3.0 2.2,18.4 4.4,19.9 6.6,8.5 8.8,14.3 11.0,16.6 13.2,15.8 15.4,10.1 17.7,9.4 19.9,19.4 22.1,14.6 24.3,5.0 26.5,14.8 28.7,15.5 30.9,7.3 33.1,18.7 35.3,20.0 37.5,12.1 39.7,8.2 41.9,9.1 44.1,14.8 46.3,13.7 48.6,17.1 50.8,17.0 53.0,15.4 55.2,13.5 57.4,7.5 59.6,8.1 61.8,7.8 64.0,15.4";
const SPARK_CACHE =
  "0.0,6.9 2.2,17.5 4.4,19.0 6.6,8.6 8.8,8.6 11.0,17.0 13.2,13.5 15.4,9.5 17.7,8.1 19.9,16.3 22.1,9.5 24.3,3.0 26.5,13.8 28.7,15.8 30.9,9.5 33.1,17.6 35.3,20.0 37.5,11.1 39.7,9.6 41.9,10.0 44.1,12.2 46.3,13.7 48.6,18.2 50.8,14.7 53.0,13.1 55.2,14.3 57.4,8.1 59.6,8.6 61.8,11.0 64.0,12.9";

/** 统计卡列表：顺序即展示顺序，value 取自 stats（与图表同一份聚合）。 */
const statCards = computed(() => {
  const s = stats.value;
  return [
    { key: "total", color: "var(--chart-total)", icon: ICON_SIGMA, spark: SPARK_TOTAL, label: t("statTotal"), value: s.total },
    { key: "requests", color: "var(--chart-requests)", icon: ICON_PULSE, spark: SPARK_REQUESTS, label: t("statRequests"), value: s.requests },
    { key: "input", color: "var(--chart-input)", icon: ICON_DOWN, spark: SPARK_INPUT, label: t("statInput"), value: s.input },
    { key: "output", color: "var(--chart-output)", icon: ICON_UP, spark: SPARK_OUTPUT, label: t("statOutput"), value: s.output },
    { key: "cache", color: "var(--chart-cache)", icon: ICON_LAYERS, spark: SPARK_CACHE, label: t("statCacheRead"), value: s.cacheRead },
  ];
});

// ---- 年度用量热力图（GitHub 贡献图式：列=周，行=周日..周六）----
// 固定近一年（365 天），不随时间范围/筛选联动；数据口径与统计卡一致：
// 日总量 = 输入 + 缓存读 + 缓存写 + 输出。只用真实记录，逐日一次遍历累加。
const HEAT_DAYS = 365;
// 列间距 = 格子宽 13px + .heat-grid-row 的 4px gap（与 CSS 保持一致，改样式需同步改这里）
const HEAT_CELL_W = 17;

/** 近一年每日分量（input/cacheRead/cacheWrite/output + requests），一次遍历 O(n) 累加。 */
const heatData = computed(() => {
  const byDay = new Map<
    string,
    { input: number; output: number; cacheRead: number; cacheWrite: number; requests: number }
  >();
  const end = new Date();
  const start = new Date(end.getFullYear(), end.getMonth(), end.getDate() - (HEAT_DAYS - 1));
  const startMs = start.getTime();
  const endMs = new Date(end.getFullYear(), end.getMonth(), end.getDate(), 23, 59, 59, 999).getTime();
  for (const r of records.value) {
    const d = new Date(r.timestamp);
    const ts = d.getTime();
    if (isNaN(ts) || ts < startMs || ts > endMs) continue;
    const k = `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
    const slot = byDay.get(k);
    if (slot) {
      slot.input += r.inputTokens;
      slot.output += r.outputTokens;
      slot.cacheRead += r.cacheReadTokens;
      slot.cacheWrite += r.cacheCreationTokens;
      slot.requests += 1;
    } else {
      byDay.set(k, {
        input: r.inputTokens,
        output: r.outputTokens,
        cacheRead: r.cacheReadTokens,
        cacheWrite: r.cacheCreationTokens,
        requests: 1,
      });
    }
  }
  return byDay;
});

interface HeatCell {
  key: string;
  blank: boolean;
  date?: Date;
  input?: number;
  output?: number;
  cacheRead?: number;
  cacheWrite?: number;
  requests?: number;
  total?: number;
}

/** 近一年的起止日与列对齐基准：范围首日所在周的周日（首列起点）。 */
const heatRange = computed(() => {
  const end = new Date();
  const start = new Date(end.getFullYear(), end.getMonth(), end.getDate() - (HEAT_DAYS - 1));
  const firstSunday = new Date(
    start.getFullYear(), start.getMonth(), start.getDate() - start.getDay(),
  );
  const lastDay = new Date(end.getFullYear(), end.getMonth(), end.getDate());
  return { start, firstSunday, lastDay };
});

/** 53 周列 × 7 行（周日开头）；首尾不满整周补 blank 占位。 */
const heatWeeks = computed(() => {
  const { start, firstSunday, lastDay } = heatRange.value;
  const out: { key: string; cells: HeatCell[] }[] = [];
  for (let w = 0; ; w++) {
    const weekStart = new Date(firstSunday.getFullYear(), firstSunday.getMonth(), firstSunday.getDate() + w * 7);
    if (weekStart.getTime() > lastDay.getTime()) break;
    const cells: HeatCell[] = [];
    for (let r = 0; r < 7; r++) {
      const d = new Date(weekStart.getFullYear(), weekStart.getMonth(), weekStart.getDate() + r);
      if (d.getTime() < start.getTime() || d.getTime() > lastDay.getTime()) {
        cells.push({ key: `b${w}-${r}`, blank: true });
        continue;
      }
      const k = `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
      const v = heatData.value.get(k) ?? { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, requests: 0 };
      cells.push({
        key: `c${w}-${r}`,
        blank: false,
        date: new Date(d.getTime()),
        input: v.input,
        output: v.output,
        cacheRead: v.cacheRead,
        cacheWrite: v.cacheWrite,
        requests: v.requests,
        total: v.input + v.output + v.cacheRead + v.cacheWrite,
      });
    }
    out.push({ key: `w${w}`, cells });
  }
  return out;
});

/** 一年内日总量峰值（分档基准；无数据兜底 1 防除零）。 */
const heatMax = computed(() => {
  let mx = 0;
  for (const w of heatWeeks.value) {
    for (const c of w.cells) {
      if (!c.blank && (c.total ?? 0) > mx) mx = c.total!;
    }
  }
  return mx || 1;
});

/** 分档：0 无数据；≤max*0.02 → 1；≤max*0.5 → 2；≤max*0.75 → 3；否则 4。 */
function heatLevel(total: number): number {
  if (!(total > 0)) return 0;
  if (total <= heatMax.value * 0.02) return 1;
  if (total <= heatMax.value * 0.5) return 2;
  if (total <= heatMax.value * 0.75) return 3;
  return 4;
}

/**
 * 月标签：落在「列首（周日）进入新月份」的那一列，绝对定位在格子行上方。
 *
 * 按列首而非「该月第一天」判定：若某月 1 号落在周尾（如周六），含它的那一列
 * 有 6 天属于上个月，标签会明显偏左、看着和下面的方块区对不齐。
 */
const heatMonthLabels = computed(() => {
  const out: { key: string; left: string; label: string }[] = [];
  const months = heatMonths();
  const first = heatRange.value.firstSunday;
  let lastYm = -1;
  heatWeeks.value.forEach((_w, wi) => {
    const sunday = new Date(first.getFullYear(), first.getMonth(), first.getDate() + wi * 7);
    const ym = sunday.getFullYear() * 12 + sunday.getMonth();
    if (ym === lastYm) return;
    lastYm = ym;
    out.push({ key: `m${ym}`, left: `${wi * HEAT_CELL_W}px`, label: months[sunday.getMonth()] });
  });
  return out;
});

/** 近一年汇总：累计请求次数 + 最长连续活跃天数（当日有请求即算活跃）。 */
const heatSummary = computed(() => {
  let requests = 0;
  let best = 0;
  let cur = 0;
  for (const w of heatWeeks.value) {
    for (const c of w.cells) {
      // 列内 7 格自上而下是周日..周六，逐列推进即为时间顺序
      if (c.blank) continue;
      if ((c.requests ?? 0) > 0) {
        requests += c.requests!;
        cur += 1;
        if (cur > best) best = cur;
      } else {
        cur = 0;
      }
    }
  }
  return { requests, best };
});

/** 热力图副标题（随数据与 locale 重算）。 */
const heatSubLabel = computed(() =>
  heatSubText(fmtNum(heatSummary.value.requests), heatSummary.value.best),
);

/** hover tooltip（position:fixed 跟随鼠标）：当日输入/缓存读/缓存写/输出/总量。 */
const heatTip = ref<{
  x: number; y: number; date: string;
  input: number; output: number; cacheRead: number; cacheWrite: number; total: number;
} | null>(null);

function heatEnter(e: MouseEvent, cell: HeatCell): void {
  if (cell.blank || !cell.date) {
    heatTip.value = null;
    return;
  }
  const d = cell.date;
  const p = (v: number) => String(v).padStart(2, "0");
  heatTip.value = {
    x: Math.min(window.innerWidth - 240, Math.max(8, e.clientX + 14)),
    y: Math.max(8, e.clientY + 16),
    date: `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`,
    input: cell.input!,
    output: cell.output!,
    cacheRead: cell.cacheRead!,
    cacheWrite: cell.cacheWrite!,
    total: cell.total!,
  };
}
function heatLeave(): void {
  heatTip.value = null;
}

// ---- 交互 ----
/** 点击列头排序：同列切换升降序，换列默认降序（时间/数字类从大到小更直观）。 */
function toggleSort(key: SortKey) {
  if (sortKey.value === key) {
    if (sortDesc.value) {
      sortDesc.value = false;
    } else {
      // 第三次点击取消排序，回到默认时间倒序
      sortKey.value = null;
      sortDesc.value = true;
    }
  } else {
    sortKey.value = key;
    sortDesc.value = true;
  }
}

/** 筛选变化时回到第 1 页。 */
function onFilterChange() {
  page.value = 1;
}

// ---- 展示辅助 ----
/** token 数值按显示模式格式化（卡片 / 表格与图表共用同一模式）。 */
function fmtNum(n: number): string {
  return fmtValue(n, fmtMode.value);
}

/** 会话展示名：标题映射优先，回退截短的 session id（子代理标注来源）。 */
function sessionTitle(r: UsageRecord): string {
  const ttl = sessionTitles.value.get(r.sessionId);
  if (ttl) return ttl;
  if (r.isSubagent || r_isSubagentId(r.sessionId)) {
    return `${t("subagentPrefix")} ${r.sessionId}`;
  }
  return r.sessionId.slice(0, 8) + "…";
}
</script>

<style scoped>
/* 用量面板：全宽单栏（不做列表+详情分栏）。面板整体纵向滚动（统计卡+图表+表格一起滚） */
.usage-panel {
  flex: 1;
  background: var(--bg-base);
  display: flex;
  flex-direction: column;
  overflow-y: auto;
  overflow-x: hidden;
  padding: var(--space-4);
  gap: var(--space-3);
  min-width: 0;
}

/* ---- 数值显示模式选择行 ---- */
.fmt-bar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-shrink: 0;
}
.fmt-select {
  width: 120px;
}

/* ---- 页头统计卡（设计稿 .stat-cards：grid 弹性换行） ---- */
.stat-cards {
  display: grid;
  /* 下限 200px：再窄则「图标 + 名称」放不下，名称会被装饰曲线挤成多行 */
  grid-template-columns: repeat(auto-fit, minmax(200px, 1fr));
  gap: var(--space-3);
  flex-shrink: 0;
}
.stat-card {
  position: relative;
  overflow: hidden;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: 14px 18px 13px;
  box-shadow: 0 1px 3px rgba(5, 8, 13, 0.45);
  display: flex;
  flex-direction: column;
  gap: 6px;
  /* 兜底色：有色卡由行内 style 覆盖，成本卡等无系列色的卡走文字色 */
  color: var(--text-3);
}
.stat-head {
  display: flex;
  align-items: center;
  gap: 10px;
  /* 右侧给绝对定位的装饰曲线留位（曲线右缘内缩 13px、宽 62px，再留 8px 间隙） */
  padding-right: 66px;
}
.stat-ico {
  width: 30px;
  height: 30px;
  border-radius: 9px;
  display: grid;
  place-items: center;
  flex: none;
  /* 图标底色 = 该卡色相的淡化版，不引入新 token */
  background: color-mix(in srgb, currentColor 14%, transparent);
}
.stat-ico svg {
  width: 15px;
  height: 15px;
  fill: none;
  stroke: currentColor;
  stroke-width: 2;
  stroke-linecap: round;
  stroke-linejoin: round;
}
.stat-name {
  font-size: 12px;
  color: var(--text-2);
  /* 英文标签更长（如 Total cache reads），允许换行保证卡高一致 */
  white-space: normal;
  line-height: 1.35;
}
.stat-value {
  font-size: 21px;
  font-weight: 800;
  color: var(--text-1);
  letter-spacing: -0.01em;
  font-variant-numeric: tabular-nums;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
/* 装饰性迷你曲线：固定点位，不接真实数据 */
.stat-spark {
  position: absolute;
  top: 15px;
  right: 13px;
  width: 62px;
  height: 22px;
}
.stat-spark polyline {
  fill: none;
  stroke: currentColor;
  stroke-width: 1.6;
  stroke-linecap: round;
  stroke-linejoin: round;
  opacity: 0.8;
}

/* ---- 趋势图 + 环形图同排（窄窗口回落为上下堆叠）
 * 默认 stretch：两张卡拉平到同一高度，趋势图的折线区用 flex:1 吃掉多出的高度。 */
.chart-row {
  display: grid;
  grid-template-columns: minmax(0, 1fr) 330px;
  gap: var(--space-3);
  flex-shrink: 0;
}
@media (max-width: 940px) {
  .chart-row {
    grid-template-columns: minmax(0, 1fr);
  }
}

/* ---- 年度用量热力图 ----
 * GitHub 贡献图式：列=周，行=周日..周六；色阶 --heat-0..4 珊瑚单色 5 档 */
.heat-card {
  flex-shrink: 0;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-3) var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}
.heat-head {
  display: flex;
  flex-direction: column;
  gap: 2px;
  margin-bottom: var(--space-2);
}
.heat-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}
.heat-sub {
  font-size: 12px;
  color: var(--text-3);
}
/* 容器横向可滚，适配窄窗口（53 列全展开 ~880px） */
.heat-scroll {
  overflow-x: auto;
  padding-bottom: var(--space-1);
}
.heat-inner {
  display: inline-flex;
  flex-direction: column;
  min-width: max-content;
}
/* 月标签列上方绝对定位 */
.heat-months {
  position: relative;
  height: 16px;
  margin-left: 34px;
}
.heat-months span {
  position: absolute;
  font-size: 10.5px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  white-space: nowrap;
}
.heat-grid-row {
  display: flex;
  align-items: flex-start;
  gap: 4px;
}
/* 周几缩写行（一/三/五，其余留空占位） */
.heat-days {
  display: grid;
  grid-template-rows: repeat(7, 13px);
  gap: 3px;
  width: 30px;
  flex-shrink: 0;
  user-select: none;
}
.heat-days span {
  font-size: 9.5px;
  line-height: 13px;
  color: var(--text-3);
  text-align: right;
  padding-right: 6px;
  font-variant-numeric: tabular-nums;
}
/* 周列：7 行格子纵向排列 */
.heat-week {
  display: grid;
  grid-template-rows: repeat(7, 13px);
  gap: 3px;
}
.heat-cell {
  width: 13px;
  height: 13px;
  border-radius: 3px;
  border: 1px solid rgba(255, 255, 255, 0.03);
  padding: 0;
  flex-shrink: 0;
}
.heat-cell.blank {
  background: transparent;
  border-color: transparent;
  pointer-events: none;
  visibility: hidden;
}
/* 图例：5 档色块 + 少/多 */
.heat-legend {
  display: flex;
  align-items: center;
  gap: 5px;
  margin-top: var(--space-2);
  font-size: 11.5px;
  color: var(--text-3);
  flex-wrap: wrap;
}
.heat-legend .lg-cell {
  width: 13px;
  height: 13px;
  border-radius: 3px;
  display: inline-block;
  border: 1px solid rgba(255, 255, 255, 0.03);
}
/* hover tooltip：fixed 跟随鼠标 */
.heat-tip {
  position: fixed;
  z-index: 80;
  pointer-events: none;
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  padding: 9px 12px;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
  display: flex;
  flex-direction: column;
  gap: 3px;
  white-space: nowrap;
  font-size: 11.5px;
  max-width: 260px;
}
.heat-tip .ht-date {
  font-size: 12px;
  color: var(--text-1);
  font-weight: 600;
  font-variant-numeric: tabular-nums;
}
.heat-tip .ht-row {
  display: flex;
  justify-content: space-between;
  gap: 16px;
  color: var(--text-3);
}
.heat-tip .ht-row b {
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
  font-weight: 700;
}
.heat-tip .ht-row.total b {
  color: var(--accent);
}

/* ---- 筛选栏 ---- */
.filter-bar {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: wrap;
  flex-shrink: 0;
}
.filter-label {
  font-size: 12px;
  color: var(--text-3);
}
.filter-select {
  width: 220px;
}
.filter-count {
  margin-left: auto;
  font-size: 12px;
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
}

/* ---- 表格 ---- */
/* 面板是整体滚动语义：统计卡/图表/表格一起在 .usage-panel 纵向滚动。
 * 表格 width:100% 自然撑开全宽，表格自身无滚动条（无嵌套滚动）；
 * 翻找靠分页器而非滚动长表。 */
.table-wrap {
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-surface);
  flex-shrink: 0;
  min-width: 0;
}
.usage-table {
  width: 100%;
  border-collapse: collapse;
  font-size: 12.5px;
  white-space: nowrap;
}
.usage-table thead th {
  background: var(--bg-elevated);
  color: var(--text-2);
  font-weight: 600;
  text-align: left;
  padding: 9px 14px;
  border-bottom: 1px solid var(--border-strong);
  user-select: none;
  font-size: 12px;
}
.usage-table th.sortable {
  cursor: pointer;
}
.usage-table th.sortable:hover {
  color: var(--text-1);
}
.usage-table th .arrow {
  display: inline-block;
  width: 12px;
  margin-left: 2px;
  color: var(--accent);
}
.usage-table tbody td {
  padding: 7px 14px;
  border-bottom: 1px solid var(--border);
  color: var(--text-2);
}
.usage-table tbody tr:hover td {
  background: var(--bg-hover);
}
/* 数字列右对齐 + 千分位（tabular-nums 保证等宽对齐） */
.usage-table .num {
  text-align: right;
  font-variant-numeric: tabular-nums;
}
.cell-time {
  color: var(--text-3);
  font-variant-numeric: tabular-nums;
  font-family: var(--font-mono);
  font-size: 11.5px;
}
/* 会话名单元格：截短展示，完整名走 NTooltip（原生 title 观感不一致已替换） */
.cell-session {
  display: inline-block;
  max-width: 220px;
  overflow: hidden;
  text-overflow: ellipsis;
  vertical-align: bottom;
  color: var(--text-1);
}
.cell-model {
  color: var(--text-1);
  font-family: var(--font-mono);
  font-size: 11.5px;
}
.cell-tier {
  color: var(--text-3);
  font-size: 11.5px;
}
.empty-cell {
  text-align: center;
  padding: 40px 0;
  color: var(--text-3);
}

/* ---- 分页 ---- */
.pager {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-3);
  flex-shrink: 0;
  padding-top: var(--space-1);
}

/* 面板底部留白收尾：滚动到底不贴边 */
.panel-tail {
  flex-shrink: 0;
  height: var(--space-6);
}
</style>
