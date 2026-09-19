<template>
  <div class="donut-card">
    <div class="donut-head">
      <div class="donut-title">{{ t("donutTitle") }}</div>
      <div class="donut-sub">{{ t("donutSub") }}</div>
    </div>

    <template v-if="hasData">
      <!-- 环形图：底色轨道 + 三段圆弧（dasharray/dashoffset 定长度与起点） -->
      <div class="donut-area">
        <svg viewBox="0 0 200 200">
          <circle class="donut-track" cx="100" cy="100" :r="R" />
          <circle
            v-for="s in slices"
            :key="s.key"
            class="donut-seg"
            cx="100"
            cy="100"
            :r="R"
            :stroke="s.color"
            :stroke-dasharray="`${s.len} ${C - s.len}`"
            :stroke-dashoffset="-s.offset"
            transform="rotate(-90 100 100)"
          >
            <!-- 原生 title：hover 显示「名称 值 · 占比」，不额外做 tooltip 层 -->
            <title>{{ s.label }} {{ fmt(s.value) }} · {{ s.pct }}</title>
          </circle>
        </svg>
        <div class="donut-center">
          <b>{{ fmt(total) }}</b>
          <span>{{ t("donutCenter") }}</span>
        </div>
      </div>

      <!-- 明细：色键 + 名称 + 值 + 占比（颜色只落在色键上，文字用文字色 token） -->
      <ul class="donut-list">
        <li v-for="s in slices" :key="s.key">
          <i :style="{ background: s.color }"></i>
          <span class="n">{{ s.label }}</span>
          <span class="v">{{ fmt(s.value) }}</span>
          <span class="p">{{ s.pct }}</span>
        </li>
      </ul>
    </template>

    <div v-else class="donut-empty">{{ t("chartEmpty") }}</div>
  </div>
</template>

<script setup lang="ts">
import { computed } from "vue";
import { fmtValue, type FmtMode } from "../usageFmt";
import { t, type I18nKey } from "../i18n";

/**
 * 用量构成环形图（纯 SVG 手写，无图表库依赖）。
 *
 * 口径与统计卡、趋势图完全一致（同为「当前时间范围 ∩ 会话/模型筛选」）：
 * 缓存命中 = cacheReadTokens，输入 = inputTokens，输出 = outputTokens。
 * 占比分母取总量（含缓存写），因此三段之和可能略小于 100%，余量为缓存写。
 *
 * 设计规范（dataviz）：扇区色取自 tokens.css 用量系列 token（与卡片图标、
 * 趋势折线同源）；身份不靠颜色单传——每个扇区带原生 <title>，右侧明细列
 * 同时给出名称、数值与占比。
 */
const props = defineProps<{
  /** Token 总量（含缓存写），占比分母 */
  total: number;
  input: number;
  output: number;
  cacheRead: number;
  fmtMode: FmtMode;
}>();

/** 半径 / 描边宽 / 周长（viewBox 200×200，圆心居中）。 */
const R = 72;
const C = 2 * Math.PI * R;

/** 扇区顺序固定：缓存命中 → 输入 → 输出（大块在前，不随数值重排）。 */
const SLICES = [
  { key: "cache", labelKey: "legendCacheHit", color: "var(--chart-cache)", field: "cacheRead" },
  { key: "input", labelKey: "legendInput", color: "var(--chart-input)", field: "input" },
  { key: "output", labelKey: "legendOutput", color: "var(--chart-output)", field: "output" },
] as const;

function fmt(n: number): string {
  return fmtValue(n, props.fmtMode);
}

const hasData = computed(() => props.total > 0);

/** 扇区几何：长度 = 占比 × 周长 - 2.5 留缝，最小 1.2 保证极小占比仍可见。 */
const slices = computed(() => {
  if (!hasData.value) return [];
  let acc = 0;
  return SLICES.map((s) => {
    const value = props[s.field];
    const frac = value / props.total;
    const len = Math.max(1.2, frac * C - 2.5);
    const offset = acc;
    acc += frac * C;
    return {
      key: s.key,
      color: s.color,
      label: t(s.labelKey as I18nKey),
      value,
      len,
      offset,
      pct: (frac * 100).toFixed(1) + "%",
    };
  });
});
</script>

<style scoped>
/* ---- 卡片外壳（与用量趋势图卡片同一套表面/边框/圆角） ---- */
.donut-card {
  flex-shrink: 0;
  background: var(--bg-surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  padding: var(--space-3) var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
}
.donut-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--text-1);
}
.donut-sub {
  font-size: 12px;
  color: var(--text-3);
  margin-top: 2px;
}

/* ---- 环形图：定宽居中，圆心叠字 ---- */
.donut-area {
  position: relative;
  width: 198px;
  margin: 4px auto var(--space-2);
}
.donut-area svg {
  display: block;
  width: 100%;
}
.donut-track {
  fill: none;
  stroke: var(--border-strong);
  stroke-width: 20;
}
.donut-seg {
  fill: none;
  stroke-width: 20;
  transition: stroke-width 0.15s ease;
}
.donut-seg:hover {
  stroke-width: 25;
}
.donut-center {
  position: absolute;
  inset: 0;
  display: grid;
  place-content: center;
  text-align: center;
}
.donut-center b {
  font-size: 21px;
  font-weight: 700;
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
}
.donut-center span {
  font-size: 11px;
  color: var(--text-3);
  margin-top: 2px;
}

/* ---- 明细列：色键 + 名称 + 值 + 占比 ---- */
.donut-list {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.donut-list li {
  display: flex;
  align-items: center;
  gap: 9px;
  font-size: 12.5px;
  padding: 7px 11px;
  border-radius: var(--radius-md);
  background: var(--bg-elevated);
}
.donut-list i {
  width: 9px;
  height: 9px;
  border-radius: 3px;
  flex: none;
}
.donut-list .n {
  color: var(--text-2);
  flex: 1;
}
.donut-list .v {
  font-weight: 600;
  color: var(--text-1);
  font-variant-numeric: tabular-nums;
}
.donut-list .p {
  color: var(--text-3);
  width: 46px;
  text-align: right;
  font-variant-numeric: tabular-nums;
}

/* ---- 空数据占位（与趋势图卡片一致的虚线框） ---- */
.donut-empty {
  display: flex;
  align-items: center;
  justify-content: center;
  min-height: 198px;
  border: 1px dashed var(--border);
  border-radius: var(--radius-md);
  font-size: 12px;
  color: var(--text-3);
}
</style>
