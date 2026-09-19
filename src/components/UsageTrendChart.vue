<template>
  <div class="chart-card">
    <div class="chart-head">
      <div class="chart-title-block">
        <div class="chart-title">{{ t("trendTitle") }}</div>
        <div class="chart-sub">{{ subLabel }}</div>
      </div>
      <div class="chart-head-right">
        <!-- 图例：色键用系列色，文字一律用文字色 token（绝不用系列色染文字） -->
        <div class="legend">
          <span v-for="s in SERIES" :key="s.key" class="legend-item">
            <span class="legend-key" :style="{ background: s.color }"></span>
            <span class="legend-text">{{ t(s.labelKey) }}</span>
          </span>
        </div>
        <!-- 时间范围：单触发器 Popover（cc-switch 式）。
             触发按钮常显当前范围；自定义态变实心色；面板内预设点击即生效，
             手写月历选自定义（不嵌 NDatePicker：其面板传送 body，嵌套会被
             外层 Popover 判定点击外部而关闭） -->
        <n-popover
          trigger="click"
          placement="bottom-end"
          :show-arrow="false"
          @update:show="onRangeOpenChange"
        >
          <template #trigger>
            <n-button
              size="tiny"
              :quaternary="range !== 'custom'"
              :secondary="range === 'custom'"
              :type="range === 'custom' ? 'primary' : 'default'"
              class="range-btn"
            >{{ triggerLabel }} ▾</n-button>
          </template>
          <div class="range-panel">
            <div class="range-presets">
              <n-button
                v-for="r in RANGE_DEFS"
                :key="r.key"
                size="tiny"
                :quaternary="range !== r.key"
                :secondary="range === r.key"
                :type="range === r.key ? 'primary' : 'default'"
                class="range-btn"
                @click="onPresetClick(r.key)"
              >{{ t(r.labelKey) }}</n-button>
            </div>
            <div class="cal-head">
              <n-button size="tiny" quaternary class="cal-nav" @click="shiftMonth(-1)">‹</n-button>
              <span class="cal-title">{{ calTitle }}</span>
              <n-button size="tiny" quaternary class="cal-nav" @click="shiftMonth(1)">›</n-button>
            </div>
            <div class="cal-week">
              <span v-for="w in WEEKDAYS" :key="w">{{ w }}</span>
            </div>
            <div class="cal-grid">
              <button
                v-for="c in calCells"
                :key="c.ts"
                type="button"
                class="cal-cell"
                :class="[
                  c.inMonth ? '' : 'dim',
                  cellState(c.ts) ?? '',
                ]"
                @click="onPickDay(c.ts)"
              >{{ c.day }}</button>
            </div>
            <div class="cal-hint">{{ pickHint }}</div>
          </div>
        </n-popover>
      </div>
    </div>

    <!-- 图表区（空数据不渲染空坐标轴，占位提示） -->
    <div ref="wrapEl" class="chart-wrap" :class="{ empty: !hasData }">
      <template v-if="hasData">
        <svg
          class="chart-svg"
          :viewBox="`0 0 ${w} ${h}`"
          @mousemove="onMove"
          @mouseleave="onLeave"
        >
          <!-- Y 轴网格（1px 实线，退后）+ 刻度文字（千分位 / tabular-nums） -->
          <g v-for="t in yTicks.ticks" :key="t">
            <line
              :x1="PAD.left"
              :x2="w - PAD.right"
              :y1="y(t)"
              :y2="y(t)"
              class="gridline"
            />
            <text
              :x="PAD.left - 8"
              :y="y(t) + 4"
              class="tick-text y-tick"
              text-anchor="end"
            >{{ fmt(t) }}</text>
          </g>

          <!-- X 轴标签（容器高度已含此带，无嵌套滚动） -->
          <text
            v-for="lb in xLabels"
            :key="lb.i"
            :x="x(lb.i)"
            :y="h - 8"
            class="tick-text"
            :text-anchor="lb.anchor"
          >{{ lb.label }}</text>

          <!-- 数据折线：2px，round join/cap；hover 行整行加亮（提示线在线上行） -->
          <path
            v-for="p in paths"
            :key="p.key"
            class="line"
            :class="{ hot: hoverIndex !== null }"
            :d="p.d"
            :style="{ stroke: p.color }"
          />

          <!-- hover 十字线（吸 nearest 数据点 X） -->
          <line
            v-if="hoverIndex !== null"
            :x1="x(hoverIndex)"
            :x2="x(hoverIndex)"
            :y1="PAD.top"
            :y2="PAD.top + plotH"
            class="crosshair"
          />

          <!-- 数据点：日常粒度 8px 圆(r4) + 2px 表面色描边环；
               5 分钟粒度（288 点）缩至 r2 防渲染过密，hover 时段放大高亮 -->
          <template v-for="p in paths" :key="'pts-' + p.key">
            <circle
              v-for="(pt, i) in series"
              :key="i"
              class="dot"
              :class="{ hot: hoverIndex === i }"
              :cx="x(i)"
              :cy="y(pt[p.key])"
              :r="hoverIndex === i ? 5 : isMinute ? 2 : 4"
              :style="{ fill: p.color }"
            />
          </template>
        </svg>

        <!-- tooltip：Vue 插值渲染（textContent 语义，无 innerHTML 拼接） -->
        <div v-if="hoverPt" class="chart-tooltip" :style="tooltipStyle">
          <div class="tip-date">{{ hoverDateLabel }}</div>
          <div v-for="row in tipRows" :key="row.key" class="tip-row">
            <span class="tip-key" :style="{ background: row.color }"></span>
            <span class="tip-value">{{ row.value }}</span>
            <span class="tip-name">{{ row.label }}</span>
          </div>
        </div>
      </template>
      <div v-else class="chart-empty">{{ t("chartEmpty") }}</div>
    </div>
  </div>
</template>

<script lang="ts">
/** 时间范围选择（状态由 UsagePanel 持有——统计卡也要用；图表消费 prop 并 emit 变更）。 */
export type RangeKey = "today" | "7d" | "14d" | "30d" | "custom";
export interface RangeSelection {
  key: RangeKey;
  /** 自定义范围毫秒（自然日：[起 00:00, 止 23:59:59.999]），非 custom 为 null */
  custom: [number, number] | null;
}
</script>

<script setup lang="ts">
import { ref, computed, onMounted, onBeforeUnmount } from "vue";
import type { UsageRecord } from "../types";
import { fmtValue, type FmtMode } from "../usageFmt";
import { t, calWeekdays, calTitle as calTitleText, type I18nKey } from "../i18n";

/**
 * 用量趋势折线图（纯 SVG 手写 + Vue computed，无图表库依赖）。
 *
 * 数据流：records（全量，时间倒序）→ 按所选时间范围过滤出自然日/5 分钟时段 →
 * 按本地时区聚合（"今天"= 5 分钟 bucket 共 288 个；近7天/近30天 = 自然日；
 * 输入 = input+cacheRead+cacheCreation 合计，输出 = outputTokens，
 * 总 = 输入+输出）→ 范围内空 bucket 补 0 → 三条 2px 折线 + hover 十字线
 * 吸最近数据点 X 的 tooltip。
 *
 * 设计规范（dataviz）：
 * 线 2px round join/cap；数据点 8px 圆 + 2px 表面色描边环；网格 1px 实线退后；
 * Y 轴刻度取整到干净数字 + 千分位 + tabular-nums；图例必有且文字用文字色 token；
 * 禁双 Y 轴；tooltip 数值 Strong、系列名次要；hover 命中吸 X 不要求点在在线上。
 */
const props = defineProps<{ records: UsageRecord[]; fmtMode: FmtMode; modelValue: RangeSelection }>();
const emit = defineEmits<{ "update:modelValue": [value: RangeSelection] }>();

// ---- 系列 ----
// 三色取自 tokens.css 用量系列 token：输入 = --chart-input（蓝）/
// 输出 = --chart-output（绿）/ 总 = --chart-total（与「Token 总量」卡同色）。
//（深色表面 #151b23 上三者色相拉开，CVD 可分；若引入亮色主题需覆盖并重新校验。）
const SERIES = [
  { key: "input", labelKey: "legendInput" as I18nKey, color: "var(--chart-input)" },
  { key: "output", labelKey: "legendOutput" as I18nKey, color: "var(--chart-output)" },
  { key: "total", labelKey: "legendTotal" as I18nKey, color: "var(--chart-total)" },
] as const;
type SeriesKey = (typeof SERIES)[number]["key"];

// ---- 时间范围（作用于统计卡与本图表，状态在 UsagePanel，图表消费 prop）----
// cc-switch 式单触发器 Popover：头部只留一个按钮，点开面板选范围。
// - 预设：点击即生效并关闭
// - 自定义：手写月历点起始日 → 点结束日 → 自动应用关闭
//   （不嵌 NDatePicker：Naive 日期面板传送 body，嵌在外层 Popover 内
//    点日历会被判定点击外部导致外层关闭）
// "今天"档位按 5 分钟 bucket 聚合，其余预设按自然日聚合。
// RangeKey / RangeSelection 类型从上方普通 <script> 导出（父组件引用）。
const RANGE_DEFS: { key: RangeKey; labelKey: I18nKey; days: number }[] = [
  { key: "today", labelKey: "rangeToday", days: 1 },
  { key: "7d", labelKey: "range7d", days: 7 },
  { key: "14d", labelKey: "range14d", days: 14 },
  { key: "30d", labelKey: "range30d", days: 30 },
];
/** 当前选择（prop 透读）。 */
const range = computed(() => props.modelValue.key);
const customRange = computed(() =>
  props.modelValue.key === "custom" ? props.modelValue.custom : null,
);

// ---- Popover 开关 + 月历草稿（每次打开重置）----
const rangeOpen = ref(false);
// 月历当前浏览月（1 号）；选择草稿起止（当日毫秒）
const calMonth = ref<Date>(new Date(new Date().getFullYear(), new Date().getMonth(), 1));
const pickStart = ref<number | null>(null);
const pickEnd = ref<number | null>(null);

function onRangeOpenChange(v: boolean): void {
  rangeOpen.value = v;
  if (v) {
    // 打开即重置草稿：回到当前月、清空已选
    const now = new Date();
    calMonth.value = new Date(now.getFullYear(), now.getMonth(), 1);
    pickStart.value = null;
    pickEnd.value = null;
  }
}

/** 月历 42 格（当月 1 号所在周起 6 周，周日开头），前后月日期淡化。 */
const calCells = computed(() => {
  const m = calMonth.value;
  const gridStart = new Date(m.getFullYear(), m.getMonth(), 1 - m.getDay());
  const out: { ts: number; day: number; inMonth: boolean }[] = [];
  for (let i = 0; i < 42; i += 1) {
    const d = new Date(gridStart.getFullYear(), gridStart.getMonth(), gridStart.getDate() + i);
    out.push({ ts: d.getTime(), day: d.getDate(), inMonth: d.getMonth() === m.getMonth() });
  }
  return out;
});

const WEEKDAYS = computed(() => calWeekdays());
const calTitle = computed(() => calTitleText(calMonth.value));

function shiftMonth(delta: number): void {
  const m = calMonth.value;
  calMonth.value = new Date(m.getFullYear(), m.getMonth() + delta, 1);
}

/** 月历选中态：起始 / 结束 / 区间内（未选或区间外为 null）。 */
function cellState(ts: number): "start" | "end" | "between" | null {
  const s = pickStart.value;
  const e = pickEnd.value;
  if (s === null) return null;
  if (ts === s) return "start";
  if (e !== null && ts === e) return "end";
  if (e !== null && ts > s && ts < e) return "between";
  return null;
}

/** 应用草稿并关闭面板。 */
function applyCustom(): void {
  if (pickStart.value === null || pickEnd.value === null) return;
  emit("update:modelValue", { key: "custom", custom: [pickStart.value, pickEnd.value] });
  rangeOpen.value = false;
}

/** 点选日期：未选/上一轮已完成 → 选起始；点同一天 → 单日范围应用；
 *  结束早于起始 → 视为重选起始；否则选结束并自动应用。 */
function onPickDay(ts: number): void {
  if (pickStart.value === null || pickEnd.value !== null) {
    pickStart.value = ts;
    pickEnd.value = null;
    return;
  }
  const d = new Date(ts);
  const dayLast = new Date(d.getFullYear(), d.getMonth(), d.getDate(), 23, 59, 59, 999).getTime();
  if (ts === pickStart.value) {
    pickEnd.value = dayLast; // 同一天：单日范围
    applyCustom();
    return;
  }
  if (ts < pickStart.value) {
    pickStart.value = ts; // 早于起始：重选起始
    return;
  }
  pickEnd.value = dayLast;
  applyCustom();
}

function onPresetClick(key: RangeKey): void {
  emit("update:modelValue", { key, custom: null });
  rangeOpen.value = false;
}

/** 触发按钮文字：预设名 / 自定义 "M/D ~ M/D"（当前范围常显）。 */
const triggerLabel = computed(() => {
  if (range.value === "custom" && customRange.value) {
    const f = (ts: number) => {
      const d = new Date(ts);
      return `${d.getMonth() + 1}/${d.getDate()}`;
    };
    return `${f(customRange.value[0])} ~ ${f(customRange.value[1])}`;
  }
  const def = RANGE_DEFS.find((d) => d.key === range.value);
  return def ? t(def.labelKey) : t("range7dShort");
});

/** 月历草稿提示（选起始 / 选结束两态；选完即应用，无第三态）。 */
const pickHint = computed(() =>
  pickStart.value === null ? t("pickStartHint") : t("pickEndHint"),
);

// 5 分钟粒度仅用于"今天"预设；自定义范围按自然日聚合（可跨月）。
// 有效天数：custom 取起止天数差（含头尾）；预设取定义值。
const rangeDays = computed(() => {
  if (range.value === "custom" && customRange.value) {
    const [s, e] = customRange.value;
    // 按自然日算天数：起始日 00:00 到结束日 00:00 的差向下取整 +1（含头尾）
    const s0 = new Date(s); s0.setHours(0, 0, 0, 0);
    const e0 = new Date(e); e0.setHours(0, 0, 0, 0);
    return Math.max(1, Math.round((e0.getTime() - s0.getTime()) / 86400000) + 1);
  }
  return RANGE_DEFS.find((d) => d.key === range.value)?.days ?? 30;
});

// ---- 聚合 ----
/** 本地时区"自然日"键（YYYY-M-D，数值不补零，避免时区与补零歧义）。 */
function localDayKey(d: Date): string {
  return `${d.getFullYear()}-${d.getMonth() + 1}-${d.getDate()}`;
}

/** 范围内每个自然日一个数据点（升序），空数据天补 0。
 *  预设：今天起往前推 N 天；custom：选择器起止日（含头尾）。
 *  数据过滤：预设只保留 <= now；custom 按起止毫秒截取。 */
const dailySeries = computed(() => {
  const now = new Date();
  const days = rangeDays.value;
  // 范围首日：custom = 起始毫秒的当日；预设 = 今天往前推 (days-1) 天
  const firstDay = (() => {
    if (range.value === "custom" && customRange.value) {
      const s = new Date(customRange.value[0]);
      return new Date(s.getFullYear(), s.getMonth(), s.getDate());
    }
    return new Date(now.getFullYear(), now.getMonth(), now.getDate() - (days - 1));
  })();
  // slot = [输入合计, 输出, 总量(输入+输出)]
  const agg = new Map<string, [number, number, number]>();
  for (const r of props.records) {
    const d = new Date(r.timestamp);
    if (isNaN(d.getTime())) continue;
    if (range.value === "custom" && customRange.value) {
      const [s, e] = customRange.value;
      if (d.getTime() < s || d.getTime() > e) continue;
    } else if (d.getTime() > now.getTime()) {
      continue;
    }
    const k = localDayKey(d);
    const slot = agg.get(k);
    const input = r.inputTokens + r.cacheReadTokens + r.cacheCreationTokens;
    if (slot) {
      slot[0] += input;
      slot[1] += r.outputTokens;
      slot[2] += input + r.outputTokens;
    } else {
      agg.set(k, [input, r.outputTokens, input + r.outputTokens]);
    }
  }

  const out: { date: Date; input: number; output: number; total: number }[] = [];
  for (let i = 0; i < days; i += 1) {
    const date = new Date(
      firstDay.getFullYear(), firstDay.getMonth(), firstDay.getDate() + i,
    );
    const v = agg.get(localDayKey(date)) ?? [0, 0, 0];
    out.push({ date, input: v[0], output: v[1], total: v[2] });
  }
  return out;
});

/** 5 分钟粒度序列（"今天" = 当天 00:00 起共 288 个 bucket）。
 *  范围内空 bucket 补 0。 */
const minuteSeries = computed(() => {
  const now = new Date();
  // 范围起点 = 当天 00:00；bucket 数 = 288（5 分钟 × 288 = 24h）
  const dayStart = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const startMs = dayStart.getTime();
  const buckets = 288;
  const endMs = startMs + (buckets - 1) * 5 * 60000;
  // key = bucket 序号；slot = [输入合计, 输出, 总量]
  const agg = new Map<number, [number, number, number]>();
  for (const r of props.records) {
    const d = new Date(r.timestamp);
    if (isNaN(d.getTime()) || d.getTime() > now.getTime()) continue;
    if (d.getTime() < startMs || d.getTime() > endMs + 5 * 60000) continue;
    // 序号 = 相对范围起点的 5 分钟 bucket 数（向下取整）
    const k = Math.floor((d.getTime() - startMs) / (5 * 60000));
    if (k < 0 || k >= buckets) continue;
    const slot = agg.get(k);
    const input = r.inputTokens + r.cacheReadTokens + r.cacheCreationTokens;
    if (slot) {
      slot[0] += input;
      slot[1] += r.outputTokens;
      slot[2] += input + r.outputTokens;
    } else {
      agg.set(k, [input, r.outputTokens, input + r.outputTokens]);
    }
  }

  const out: { date: Date; input: number; output: number; total: number }[] = [];
  for (let i = 0; i < buckets; i += 1) {
    const date = new Date(startMs + i * 5 * 60000);
    const v = agg.get(i) ?? [0, 0, 0];
    out.push({ date, input: v[0], output: v[1], total: v[2] });
  }
  return out;
});

// 当前时间范围的展示序列（"今天" = 5 分钟粒度，其余 = 自然日）
const isMinute = computed(() => range.value === "today");
const series = computed(() =>
  isMinute.value ? minuteSeries.value : dailySeries.value,
);

const hasData = computed(() =>
  series.value.some((d) => d.input > 0 || d.output > 0),
);

/** 副标题随粒度变化（"今天"= 5 分钟，其余 = 自然日）。 */
const subLabel = computed(() =>
  isMinute.value ? t("trendSubMin") : t("trendSubDay"),
);

// ---- 几何（宽高都随容器实测，viewBox 与容器 1:1 无拉伸变形）----
const PAD = { top: 16, right: 16, bottom: 28, left: 64 };
const wrapEl = ref<HTMLElement | null>(null);
const w = ref(900);
const h = ref(260);
const plotW = computed(() => Math.max(100, w.value - PAD.left - PAD.right));
const plotH = computed(() => Math.max(60, h.value - PAD.top - PAD.bottom));

let ro: ResizeObserver | null = null;
onMounted(() => {
  if (!wrapEl.value) return;
  const sync = (width: number, height: number): void => {
    w.value = Math.max(320, Math.round(width));
    h.value = Math.max(200, Math.round(height));
  };
  sync(wrapEl.value.clientWidth, wrapEl.value.clientHeight);
  ro = new ResizeObserver((entries) => {
    // 高度实测而非写死：本卡与环形图同排等高，被撑高时折线区跟着长
    for (const e of entries) sync(e.contentRect.width, e.contentRect.height);
  });
  ro.observe(wrapEl.value);
});
onBeforeUnmount(() => ro?.disconnect());

/** 干净刻度步长：1/2/2.5/5 × 10^n。 */
function niceStep(rough: number): number {
  if (rough <= 0) return 1;
  const exp = Math.floor(Math.log10(rough));
  const base = Math.pow(10, exp);
  const frac = rough / base;
  const mult = frac <= 1 ? 1 : frac <= 2 ? 2 : frac <= 2.5 ? 2.5 : frac <= 5 ? 5 : 10;
  return mult * base;
}

/** Y 轴刻度：0 起步，上限向上取整到步长整数倍（0 / 50,000 / 100,000 这类干净数）。 */
const yTicks = computed(() => {
  // 总量恒 >= 输入/输出单项，max 取三系列极值（total 恒在最上方）
  const max = Math.max(
    1,
    ...series.value.map((d) => Math.max(d.input, d.output, d.total)),
  );
  const step = niceStep(max / 4);
  const top = Math.ceil(max / step) * step;
  const ticks: number[] = [];
  for (let v = 0; v <= top + 1e-9; v += step) ticks.push(v);
  return { ticks, top };
});

function x(i: number): number {
  const n = series.value.length;
  if (n <= 1) return PAD.left + plotW.value / 2;
  return PAD.left + (i / (n - 1)) * plotW.value;
}
function y(v: number): number {
  const top = yTicks.value.top || 1;
  return PAD.top + plotH.value - (v / top) * plotH.value;
}

/**
 * 单调三次插值（Fritsch–Carlson）转 bezier path：
 * 逐段限制切线，曲线不过冲越界（极值点间无上冲/下穿，数据值之上/之下不越界）。
 * n < 2 时回退空串（配合外层空数据 v-if 不渲染）。
 */
function smoothPath(xs: number[], ys: number[]): string {
  const n = xs.length;
  if (n < 2) return "";
  if (n === 2) {
    return `M${xs[0].toFixed(1)},${ys[0].toFixed(1)} L${xs[1].toFixed(1)},${ys[1].toFixed(1)}`;
  }
  const m = new Array<number>(n);
  const d = new Array<number>(n - 1);
  const s = new Array<number>(n - 1);
  for (let i = 0; i < n - 1; i++) {
    d[i] = xs[i + 1] - xs[i] || 1e-9;
    s[i] = (ys[i + 1] - ys[i]) / d[i];
  }
  m[0] = s[0];
  m[n - 1] = s[n - 2];
  for (let i = 1; i < n - 1; i++) {
    m[i] = s[i - 1] * s[i] <= 0 ? 0 : (s[i - 1] + s[i]) / 2;
  }
  // 逐段限制切线（monotonicity constraint）：|m/s| 向量长度 > 3 时缩到 3
  for (let i = 0; i < n - 1; i++) {
    if (s[i] === 0) {
      m[i] = 0;
      m[i + 1] = 0;
      continue;
    }
    const a = m[i] / s[i];
    const b = m[i + 1] / s[i];
    const h = Math.sqrt(a * a + b * b);
    if (h > 3) {
      const t = 3 / h;
      m[i] = t * a * s[i];
      m[i + 1] = t * b * s[i];
    }
  }
  let out = `M${xs[0].toFixed(1)},${ys[0].toFixed(1)}`;
  for (let i = 0; i < n - 1; i++) {
    const h3 = d[i] / 3;
    out +=
      ` C${(xs[i] + h3).toFixed(1)},${(ys[i] + m[i] * h3).toFixed(1)}` +
      ` ${(xs[i + 1] - h3).toFixed(1)},${(ys[i + 1] - m[i + 1] * h3).toFixed(1)}` +
      ` ${xs[i + 1].toFixed(1)},${ys[i + 1].toFixed(1)}`;
  }
  return out;
}

/** 平滑曲线 path（Fritsch–Carlson 单调三次插值；覆盖范围内每个 bucket，含补 0）。 */
const paths = computed(() =>
  SERIES.map((s) => {
    const xs = series.value.map((_, i) => x(i));
    const ys = series.value.map((pt) => y(pt[s.key]));
    return { ...s, d: smoothPath(xs, ys) };
  }),
);

/**
 * X 轴标签抽样：
 * - 5 分钟粒度（今天 288 点）：只标整点刻度，今天 24 个；
 * - 近7天：逐日；近30天：每 3 天一个；且最后一个 bucket 必标（右端收口）。
 */
const xLabels = computed(() => {
  const n = series.value.length;
  if (n === 0) return [];
  const isMin = isMinute.value;
  const stepEvery = isMin ? 12 : rangeDays.value <= 7 ? 1 : 3; // 5min×12 = 整点
  const p2 = (v: number) => String(v).padStart(2, "0");
  const mk = (i: number) => {
    const d = series.value[i].date;
    return {
      i,
      label: isMin
        ? `${p2(d.getHours())}:${p2(d.getMinutes())}`
        : `${d.getMonth() + 1}/${d.getDate()}`,
      anchor: i === 0 ? "start" : i === n - 1 ? "end" : "middle",
    };
  };
  const out = [];
  for (let i = 0; i < n; i += stepEvery) out.push(mk(i));
  if (!isMin) {
    // 近7/近30天：最后一个 bucket 必标（右端收口）。
    // 今天不补 23:55：与 23:00 整点刻度只差 31px，end 锚点会重叠，整点刻度已框住全天。
    const last = n - 1;
    if (out[out.length - 1].i !== last) out.push(mk(last));
  }
  return out;
});

// ---- hover：十字线吸 nearest 数据点 X，tooltip 显示该时段各系列 ----
const hoverIndex = ref<number | null>(null);

function onMove(e: MouseEvent): void {
  const svg = e.currentTarget as SVGSVGElement;
  const rect = svg.getBoundingClientRect();
  const n = series.value.length;
  if (n === 0) return;
  // viewBox 与容器 1:1（宽度实测），直接用像素坐标
  const px = e.clientX - rect.left;
  const idx = Math.round(((px - PAD.left) / plotW.value) * (n - 1));
  hoverIndex.value = Math.max(0, Math.min(n - 1, idx));
}
function onLeave(): void {
  hoverIndex.value = null;
}

const hoverPt = computed(() =>
  hoverIndex.value === null ? null : series.value[hoverIndex.value],
);

/** tooltip 定位：X 贴十字线（1:1 像素），贴边翻转。 */
const tooltipStyle = computed(() => {
  const i = hoverIndex.value;
  if (i === null || !wrapEl.value) return { left: "0px" };
  const width = wrapEl.value.clientWidth;
  const TOOLTIP_HALF = 80; // 预估半宽，仅用于贴边翻转
  const leftPx = Math.min(
    width - TOOLTIP_HALF,
    Math.max(TOOLTIP_HALF, x(i)),
  );
  return { left: `${leftPx}px` };
});

const hoverDateLabel = computed(() => {
  const pt = hoverPt.value;
  if (!pt) return "";
  const d = pt.date;
  const p = (n: number) => String(n).padStart(2, "0");
  if (isMinute.value) {
    // 5 分钟粒度：显示"HH:MM 时段"（bucket 的起始时刻，覆盖其后 5 分钟）
    return `${p(d.getHours())}:${p(d.getMinutes())}${t("hoverSlotPost")}`;
  }
  const now = new Date();
  const isToday =
    d.getFullYear() === now.getFullYear() &&
    d.getMonth() === now.getMonth() &&
    d.getDate() === now.getDate();
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}${isToday ? t("hoverToday") : ""}`;
});

/** tooltip 数值按显示模式格式化（与 Y 轴刻度共用同一函数）。 */
function fmt(n: number): string {
  return fmtValue(n, props.fmtMode);
}

/** tooltip 行（数值 Strong 在前，系列名次要）。 */
const tipRows = computed(() => {
  const pt = hoverPt.value;
  if (!pt) return [];
  return SERIES.map((s) => ({
    key: s.key as SeriesKey,
    color: s.color,
    label: t(s.labelKey),
    value: fmt(pt[s.key]),
  }));
});
</script>

<style scoped>
/* ---- 图表卡片 ----
 * 三系列色统一走 tokens.css 的 --chart-input / --chart-output / --chart-cache */
.chart-card {
  flex-shrink: 0;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-3) var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.chart-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: var(--space-3);
  flex-wrap: wrap;
}
.chart-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}
.chart-sub {
  font-size: 12px;
  color: var(--text-3);
  margin-top: 2px;
}
.chart-head-right {
  display: flex;
  align-items: center;
  gap: var(--space-4);
  flex-wrap: wrap;
}

/* ---- 图例：线段色键 + 文字色文本 ---- */
.legend {
  display: flex;
  gap: var(--space-3);
}
.legend-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
}
.legend-key {
  width: 14px;
  height: 3px;
  border-radius: 2px;
  display: inline-block;
}
.legend-text {
  font-size: 12px;
  color: var(--text-2);
}

/* ---- 时间范围：单触发器 Popover ----
 * 触发按钮常显当前范围；预设/自定义选中 = secondary primary（品牌色底） */
.range-btn {
  font-size: 12px;
}

/* Popover 内面板（预设行 + 手写月历） */
.range-panel {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  padding: 2px;
}
.range-presets {
  display: flex;
  gap: var(--space-1);
  flex-wrap: wrap;
  padding-bottom: var(--space-2);
  border-bottom: 1px solid var(--border);
}

/* 月历：标题行 + 星期行 + 42 格（7×6） */
.cal-head {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
}
.cal-nav {
  padding: 0 6px;
}
.cal-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text-1);
  min-width: 90px;
  text-align: center;
}
.cal-week {
  display: grid;
  grid-template-columns: repeat(7, 30px);
}
.cal-week span {
  font-size: 11px;
  color: var(--text-3);
  text-align: center;
}
.cal-grid {
  display: grid;
  grid-template-columns: repeat(7, 30px);
}
.cal-cell {
  width: 28px;
  height: 28px;
  margin: 1px;
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-2);
  font-size: 12px;
  cursor: pointer;
  font-variant-numeric: tabular-nums;
}
.cal-cell:hover {
  background: var(--bg-hover);
  color: var(--text-1);
}
/* 前后月日期淡化 */
.cal-cell.dim {
  color: var(--text-3);
  opacity: 0.45;
}
/* 起止与区间内：品牌色底；起止实心、区间半透明 */
.cal-cell.start,
.cal-cell.end {
  background: var(--accent);
  color: var(--on-accent);
  font-weight: 600;
}
.cal-cell.between {
  background: color-mix(in srgb, var(--accent) 18%, transparent);
  color: var(--text-1);
}
.cal-hint {
  font-size: 11px;
  color: var(--text-3);
  text-align: center;
}

/* ---- 图表区：高度含 X 轴标签带，无嵌套滚动 ----
 * flex:1 让折线区吃掉卡片剩余高度：卡片与环形图同排等高被撑高时，
 * 图表跟着长（高度由 ResizeObserver 实测进 viewBox，不写死）。 */
.chart-wrap {
  position: relative;
  flex: 1;
  min-height: 260px;
  user-select: none;
}
.chart-svg {
  width: 100%;
  height: 100%;
  display: block;
  cursor: crosshair;
}
.chart-wrap.empty {
  display: flex;
  align-items: center;
  justify-content: center;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
}
.chart-empty {
  font-size: 12px;
  color: var(--text-3);
}

/* 网格 / 十字线：1px 实线（禁虚线），比表面浅一阶灰，退后不抢数据 */
.gridline {
  stroke: var(--border);
  stroke-width: 1;
  shape-rendering: crispEdges;
}
.crosshair {
  stroke: var(--border-strong);
  stroke-width: 1;
}
.tick-text {
  font-size: 11px;
  fill: var(--text-3);
}
.y-tick {
  font-variant-numeric: tabular-nums;
}

/* 折线：2px round join/cap；hover 提亮 */
.line {
  fill: none;
  stroke-width: 2;
  stroke-linejoin: round;
  stroke-linecap: round;
}
.line.hot {
  filter: brightness(1.25);
}

/* 数据点：8px 圆(r4) + 2px 表面色描边环；hover 天放大至 10px 并提亮 */
.dot {
  stroke: var(--bg-surface);
  stroke-width: 2;
}
.dot.hot {
  filter: brightness(1.25);
}

/* ---- tooltip：数值 Strong、系列名次要 ---- */
.chart-tooltip {
  position: absolute;
  top: 8px;
  transform: translateX(-50%);
  background: var(--bg-elevated);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-md);
  padding: 8px 12px;
  pointer-events: none;
  white-space: nowrap;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.35);
  display: flex;
  flex-direction: column;
  gap: 4px;
  z-index: 2;
}
.tip-date {
  font-size: 12px;
  color: var(--text-2);
  font-variant-numeric: tabular-nums;
}
.tip-row {
  display: flex;
  align-items: baseline;
  gap: 6px;
}
.tip-key {
  width: 12px;
  height: 3px;
  border-radius: 2px;
  align-self: center;
  flex-shrink: 0;
}
.tip-value {
  font-size: 13px;
  font-weight: 700;
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
}
.tip-name {
  font-size: 12px;
  color: var(--text-3);
}
</style>
