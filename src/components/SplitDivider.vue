<template>
  <div
    ref="root"
    class="split-divider"
    :class="{ dragging }"
    @pointerdown="onPointerDown"
  ></div>
</template>

<script setup lang="ts">
import { onMounted, onUnmounted, ref } from "vue";

/**
 * 列表/详情两栏之间的可拖拽分隔线。
 * 拖动实时改变列表宽度（v-model 同步给父级，绑定到列表面板的 style.width），
 * 松手后按 storageKey 持久化到 localStorage，各界面位置独立记忆。
 */
const props = withDefaults(
  defineProps<{
    /** localStorage 持久化 key（按界面区分） */
    storageKey: string;
    /** 无历史记录时的列表默认宽度（px） */
    defaultWidth: number;
    /** 列表最小宽度（px） */
    minList?: number;
    /** 详情最小宽度（px） */
    minDetail?: number;
  }>(),
  { minList: 200, minDetail: 300 }
);

const width = defineModel<number>({ required: true });

const dragging = ref(false);
const root = ref<HTMLElement | null>(null);

/** 把宽度约束到 [minList, 容器宽 - 分隔线 - minDetail]，窗口缩小时位置收敛不破坏布局。 */
function clampToViewport(w: number): number {
  const avail =
    (root.value?.parentElement?.clientWidth ?? 0) - (root.value?.offsetWidth ?? 0);
  if (avail > 0) {
    const max = Math.max(props.minList, avail - props.minDetail);
    w = Math.min(w, max);
  }
  return Math.max(props.minList, w);
}

// ---- 拖动：pointer 事件，move/up 挂在 document 上，结束后清理 ----
let startX = 0;
let startWidth = 0;

function onPointerMove(e: PointerEvent) {
  width.value = clampToViewport(startWidth + (e.clientX - startX));
}

function endDrag() {
  dragging.value = false;
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
  document.removeEventListener("pointermove", onPointerMove);
  document.removeEventListener("pointerup", endDrag);
  document.removeEventListener("pointercancel", endDrag);
  persist();
}

function onPointerDown(e: PointerEvent) {
  e.preventDefault();
  dragging.value = true;
  startX = e.clientX;
  startWidth = width.value;
  document.body.style.cursor = "col-resize";
  document.body.style.userSelect = "none";
  // 捕获指针：鼠标拖出窗口外也能持续收到 move/up（失败时靠 document 监听兜底）
  try {
    root.value?.setPointerCapture(e.pointerId);
  } catch {
    /* 无效 pointerId 时忽略，document 监听已兜底 */
  }
  document.addEventListener("pointermove", onPointerMove);
  document.addEventListener("pointerup", endDrag);
  document.addEventListener("pointercancel", endDrag);
}

function persist() {
  try {
    localStorage.setItem(props.storageKey, String(Math.round(width.value)));
  } catch {
    /* localStorage 不可用时静默降级为仅本次会话内生效 */
  }
}

/** 窗口尺寸变化时收敛到约束范围内（不回写存储，窗口恢复后位置还原）。 */
function onWindowResize() {
  width.value = clampToViewport(width.value);
}

onMounted(() => {
  // 初始化：优先读历史位置，再按当前容器宽度收敛（历史值可能超出小窗口）
  let stored = 0;
  try {
    stored = Number(localStorage.getItem(props.storageKey));
  } catch {
    /* 读取失败时用默认宽度 */
  }
  width.value =
    Number.isFinite(stored) && stored > 0 ? stored : props.defaultWidth;
  width.value = clampToViewport(width.value);
  window.addEventListener("resize", onWindowResize);
});

onUnmounted(() => {
  window.removeEventListener("resize", onWindowResize);
  document.removeEventListener("pointermove", onPointerMove);
  document.removeEventListener("pointerup", endDrag);
  document.removeEventListener("pointercancel", endDrag);
  // 视图切换在拖动中卸载时同样还原全局光标
  document.body.style.cursor = "";
  document.body.style.userSelect = "";
});
</script>
