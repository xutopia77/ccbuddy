/**
 * 用量数值显示模式（整个用量界面共用：统计卡 / 图表 Y 轴+tooltip / 表格）。
 *
 * - raw：千分位原始数
 * - si：K/M/G 缩写（1e3/1e6/1e9）
 * - cn：中文单位 万/亿/万亿（1e4/1e8/1e12）
 */
export type FmtMode = "raw" | "si" | "cn";

import { t } from "./i18n";

/** 下拉选项（label 随语言切换；cn 档单位 万/亿 是中文数值档位本身，不译） */
export function fmtOptions(): { label: string; value: FmtMode }[] {
  return [
    { label: t("fmtRaw"), value: "raw" },
    { label: "K/M/G", value: "si" },
    { label: "万/亿", value: "cn" },
  ];
}

/** 数值按显示模式格式化：缩写/中文单位时保留至多 1 位小数（去尾零）。 */
export function fmtValue(v: number, mode: FmtMode): string {
  if (mode === "raw" || v === 0) return v.toLocaleString("en-US");
  const trim = (n: number): string => {
    // 大数不保留小数位（123.4M 已足够长），小数保留 1 位去尾零
    const s = n >= 100 ? n.toFixed(0) : n.toFixed(1);
    return s.endsWith(".0") ? s.slice(0, -2) : s;
  };
  if (mode === "si") {
    if (v >= 1e9) return `${trim(v / 1e9)}G`;
    if (v >= 1e6) return `${trim(v / 1e6)}M`;
    if (v >= 1e3) return `${trim(v / 1e3)}K`;
    return String(v);
  }
  if (v >= 1e12) return `${trim(v / 1e12)}万亿`;
  if (v >= 1e8) return `${trim(v / 1e8)}亿`;
  if (v >= 1e4) return `${trim(v / 1e4)}万`;
  return String(v);
}
