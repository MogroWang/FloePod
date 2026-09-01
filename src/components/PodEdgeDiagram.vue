<script setup lang="ts">
/**
 * 位置状态示意图：圆角矩形代表屏幕，靠近哪条边蓝点就贴哪条边。
 * 小蓝点可以随意拖动：拖到另一条边附近即切换所在边，同时更新沿边位置
 * （也支持方向键微调，Shift 加速）。拖动过程实时上报 update:offset 预览，
 * 松手后上报 commit 保存；切换边时立即上报 update:edge 由宿主保存。
 */
import { computed, ref } from "vue";

const props = defineProps<{
  edge: string; // top / right / bottom / left
  offset: number; // 0.0 - 1.0
  monitorLabel?: string;
}>();

const emit = defineEmits<{
  (e: "update:edge", value: string): void;
  (e: "update:offset", value: number): void;
  (e: "commit", value: number): void;
}>();

const EDGE_LABELS: Record<string, string> = {
  top: "上",
  right: "右",
  bottom: "下",
  left: "左",
};

const VERTICAL_EDGES = new Set(["left", "right"]);

const screenEl = ref<HTMLElement | null>(null);
const dragging = ref(false);
/** 拖动中的所在边：跟随指针即时变化，不等宿主保存回流。 */
const dragEdge = ref(props.edge);

const displayEdge = computed(() => (dragging.value ? dragEdge.value : props.edge));
const pct = computed(() => Math.round(props.offset * 100));

const dotStyle = computed<Record<string, string>>(() => {
  const p = `${pct.value}%`;
  switch (displayEdge.value) {
    case "top":
      return { left: p, top: "0" };
    case "bottom":
      return { left: p, top: "100%" };
    case "right":
      return { left: "100%", top: p };
    default:
      return { left: "0", top: p };
  }
});

const caption = computed(() => {
  const parts = [EDGE_LABELS[displayEdge.value] ?? "?", `${pct.value}%`];
  if (props.monitorLabel) parts.push(props.monitorLabel);
  return parts.join(" · ");
});

/* --- 蓝点拖动定位（可跨边切换） --- */

function clampOffset(value: number): number {
  return Math.round(Math.min(1, Math.max(0, value)) * 100) / 100;
}

/** 指针落在哪条边的感应区（每条边 18% 的带状区域）；中间区域返回 null 不切边。 */
function edgeUnderPointer(rect: DOMRect, x: number, y: number): string | null {
  const band = 0.18;
  if (y <= rect.height * band) return "top";
  if (y >= rect.height * (1 - band)) return "bottom";
  if (x <= rect.width * band) return "left";
  if (x >= rect.width * (1 - band)) return "right";
  return null;
}

function projectedOffset(rect: DOMRect, edge: string, x: number, y: number): number {
  return VERTICAL_EDGES.has(edge)
    ? clampOffset(y / rect.height)
    : clampOffset(x / rect.width);
}

function onDotPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  const el = screenEl.value;
  if (!el) return;
  dragging.value = true;
  dragEdge.value = props.edge;
  // 捕获指针：拖出示意图边界后仍能继续调整，松手前事件始终派发给蓝点。
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  const rect = el.getBoundingClientRect();
  emit(
    "update:offset",
    projectedOffset(rect, dragEdge.value, event.clientX - rect.left, event.clientY - rect.top),
  );
  event.preventDefault();
}

function onDotPointerMove(event: PointerEvent) {
  if (!dragging.value) return;
  const el = screenEl.value;
  if (!el) return;
  const rect = el.getBoundingClientRect();
  const x = event.clientX - rect.left;
  const y = event.clientY - rect.top;
  const edge = edgeUnderPointer(rect, x, y);
  if (edge && edge !== dragEdge.value) {
    dragEdge.value = edge;
    emit("update:edge", edge);
  }
  emit("update:offset", projectedOffset(rect, dragEdge.value, x, y));
}

function onDotPointerUp(event: PointerEvent) {
  if (!dragging.value) return;
  dragging.value = false;
  const el = event.currentTarget as HTMLElement | null;
  if (el?.hasPointerCapture(event.pointerId)) el.releasePointerCapture(event.pointerId);
  const rect = screenEl.value?.getBoundingClientRect();
  const offset = rect
    ? projectedOffset(rect, dragEdge.value, event.clientX - rect.left, event.clientY - rect.top)
    : props.offset;
  emit("commit", offset);
}

/** 键盘微调：方向键 ±1%，Shift 加速到 ±5%。 */
function onDotKeydown(event: KeyboardEvent) {
  const step = event.shiftKey ? 0.05 : 0.01;
  let delta = 0;
  if (event.key === "ArrowUp" || event.key === "ArrowLeft") delta = -step;
  else if (event.key === "ArrowDown" || event.key === "ArrowRight") delta = step;
  else return;
  event.preventDefault();
  const next = clampOffset(props.offset + delta);
  if (next !== props.offset) {
    emit("update:offset", next);
    emit("commit", next);
  }
}
</script>

<template>
  <div class="edge-diagram">
    <div ref="screenEl" class="screen">
      <span class="edge e-top" :class="{ on: displayEdge === 'top' }" />
      <span class="edge e-right" :class="{ on: displayEdge === 'right' }" />
      <span class="edge e-bottom" :class="{ on: displayEdge === 'bottom' }" />
      <span class="edge e-left" :class="{ on: displayEdge === 'left' }" />
      <button
        type="button"
        class="dot"
        :class="{ dragging }"
        :style="dotStyle"
        :aria-label="`拖动定位，可拖到任意屏幕边缘；当前 ${caption}`"
        @pointerdown="onDotPointerDown"
        @pointermove="onDotPointerMove"
        @pointerup="onDotPointerUp"
        @pointercancel="onDotPointerUp"
        @keydown="onDotKeydown"
      />
    </div>
    <div class="caption">{{ caption }}</div>
  </div>
</template>

<style scoped>
.edge-diagram {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 8px;
  padding: 4px 0 10px;
}
.screen {
  position: relative;
  width: 200px;
  height: 128px;
  border-radius: 12px;
  border: 1.5px solid var(--line-strong);
  background: var(--surface-hover);
  box-shadow: inset 0 1px 0 oklch(1 0 0 / 0.35);
}
.edge {
  position: absolute;
  background: var(--line);
  transition: background 160ms var(--ease-out), height 160ms var(--ease-out),
    width 160ms var(--ease-out);
}
.edge.on {
  background: var(--accent);
}
.e-top {
  top: 0;
  left: 10%;
  right: 10%;
  height: 3px;
}
.e-bottom {
  bottom: 0;
  left: 10%;
  right: 10%;
  height: 3px;
}
.e-left {
  left: 0;
  top: 12%;
  bottom: 12%;
  width: 3px;
}
.e-right {
  right: 0;
  top: 12%;
  bottom: 12%;
  width: 3px;
}
.dot {
  position: absolute;
  width: 14px;
  height: 14px;
  padding: 0;
  border: 0;
  border-radius: 50%;
  background: var(--accent);
  box-shadow: 0 0 0 2px var(--surface-raised), 0 2px 6px oklch(0 0 0 / 0.25);
  transform: translate(-50%, -50%);
  cursor: grab;
  touch-action: none;
  transition: left 160ms var(--ease-out), top 160ms var(--ease-out),
    transform 100ms var(--ease-out);
}
.dot:hover {
  transform: translate(-50%, -50%) scale(1.15);
}
.dot:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}
/* 拖动中跟随指针实时定位，禁用过渡与放大反馈 */
.dot.dragging {
  cursor: grabbing;
  transition: none;
  transform: translate(-50%, -50%) scale(1.25);
  box-shadow: 0 0 0 3px var(--surface-raised), 0 3px 10px oklch(0 0 0 / 0.3);
}
.dot.dragging:hover {
  transform: translate(-50%, -50%) scale(1.25);
}
.caption {
  font-size: 11.5px;
  color: var(--ink-3);
  font-variant-numeric: tabular-nums;
  letter-spacing: 0.01em;
}
</style>
