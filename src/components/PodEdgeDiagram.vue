<script setup lang="ts">
/**
 * 位置状态示意图：圆角矩形代表屏幕，当前边高亮，
 * 小蓝点表示匣的位置，可直接沿高亮边拖动定位（也支持方向键微调）。
 * 拖动过程实时上报 update:offset 供预览，松手（或按键）后上报 commit 保存，
 * 与「沿边缘位置」滑杆共用同一份草稿状态。
 */
import { computed, ref } from "vue";

const props = defineProps<{
  edge: string; // top / right / bottom / left
  offset: number; // 0.0 - 1.0
  monitorLabel?: string;
}>();

const emit = defineEmits<{
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

const pct = computed(() => Math.round(props.offset * 100));
const vertical = computed(() => VERTICAL_EDGES.has(props.edge));

const dotStyle = computed<Record<string, string>>(() => {
  const p = `${pct.value}%`;
  switch (props.edge) {
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
  const parts = [EDGE_LABELS[props.edge] ?? "?", `${pct.value}%`];
  if (props.monitorLabel) parts.push(props.monitorLabel);
  return parts.join(" · ");
});

/* --- 小蓝点拖动定位 --- */
const screenEl = ref<HTMLElement | null>(null);
const dragging = ref(false);

function clampOffset(value: number): number {
  return Math.round(Math.min(1, Math.max(0, value)) * 100) / 100;
}

function offsetFromPointer(event: PointerEvent): number {
  const el = screenEl.value;
  if (!el) return props.offset;
  const rect = el.getBoundingClientRect();
  if (rect.width <= 0 || rect.height <= 0) return props.offset;
  const raw = vertical.value
    ? (event.clientY - rect.top) / rect.height
    : (event.clientX - rect.left) / rect.width;
  return clampOffset(raw);
}

function onDotPointerDown(event: PointerEvent) {
  if (event.button !== 0) return;
  dragging.value = true;
  // 捕获指针：拖出示意图边界后仍能继续调整，松手前事件始终派发给蓝点。
  (event.currentTarget as HTMLElement).setPointerCapture(event.pointerId);
  emit("update:offset", offsetFromPointer(event));
  event.preventDefault();
}

function onDotPointerMove(event: PointerEvent) {
  if (!dragging.value) return;
  emit("update:offset", offsetFromPointer(event));
}

function onDotPointerUp(event: PointerEvent) {
  if (!dragging.value) return;
  dragging.value = false;
  const el = event.currentTarget as HTMLElement | null;
  if (el?.hasPointerCapture(event.pointerId)) el.releasePointerCapture(event.pointerId);
  emit("commit", offsetFromPointer(event));
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
      <span class="edge e-top" :class="{ on: edge === 'top' }" />
      <span class="edge e-right" :class="{ on: edge === 'right' }" />
      <span class="edge e-bottom" :class="{ on: edge === 'bottom' }" />
      <span class="edge e-left" :class="{ on: edge === 'left' }" />
      <button
        type="button"
        class="dot"
        :class="{ dragging }"
        :style="dotStyle"
        :aria-label="`沿边缘拖动定位，当前 ${caption}`"
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
