<script setup lang="ts">
/**
 * 条目行：缩略图 + 名称 + 元信息；悬停显示操作；
 * 拖拽（移动超过阈值）发起 OS 拖出；点选 / Ctrl 点选 / Shift 范围选；双击打开。
 */
import { computed, onBeforeUnmount, ref } from "vue";
import type { StagedItem } from "@/domain/types";
import { formatSize, formatTime, kindLabel } from "@/lib/format";
import ThumbImg from "./ThumbImg.vue";

const props = defineProps<{
  item: StagedItem;
  selected: boolean;
  getDragPaths: () => string[];
}>();

const emit = defineEmits<{
  (e: "select", id: number, mode: "set" | "toggle" | "range"): void;
  (e: "open", item: StagedItem): void;
  (e: "reveal", item: StagedItem): void;
  (e: "remove", item: StagedItem): void;
  (e: "dragOut", paths: string[]): void;
}>();

const dragArmed = ref(false);
let stopPointerTracking: (() => void) | null = null;

const meta = computed(() => {
  const parts: string[] = [];
  if (item_kind.value === "text") parts.push("文字");
  else parts.push(kindLabel(props.item.kind));
  if (props.item.size > 0) parts.push(formatSize(props.item.size));
  parts.push(formatTime(props.item.createdAt));
  return parts.join(" · ");
});

const item_kind = computed(() => props.item.kind);

function onPointerDown(e: PointerEvent) {
  if (e.button !== 0) return;
  const target = e.target as HTMLElement;
  if (target.closest("button")) return;
  // 原生拖出偶尔会补发一次合成点击：短暂保留标记吸收它，下一次真实按下时再清除。
  dragArmed.value = false;
  const startX = e.clientX;
  const startY = e.clientY;
  let reported = false;
  const pointerId = e.pointerId;

  const move = (ev: PointerEvent) => {
    if (ev.pointerId !== pointerId) return;
    const dx = ev.clientX - startX;
    const dy = ev.clientY - startY;
    if (!reported && Math.hypot(dx, dy) > 6) {
      reported = true;
      dragArmed.value = true;
      emit("dragOut", props.getDragPaths());
      cleanup();
    }
  };
  const cleanup = () => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", cleanup);
    window.removeEventListener("pointercancel", cleanup);
    window.removeEventListener("blur", cleanup);
    stopPointerTracking = null;
  };
  stopPointerTracking?.();
  stopPointerTracking = cleanup;
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", cleanup);
  window.addEventListener("pointercancel", cleanup);
  window.addEventListener("blur", cleanup);
}

function onClick(e: MouseEvent) {
  if (dragArmed.value) {
    dragArmed.value = false;
    return;
  }
  const mode = e.shiftKey ? "range" : e.ctrlKey ? "toggle" : "set";
  emit("select", props.item.id, mode);
}

function onKeydown(e: KeyboardEvent) {
  if ((e.target as HTMLElement).closest("button")) return;
  if (e.key === "Enter") {
    e.preventDefault();
    emit("open", props.item);
  } else if (e.key === " ") {
    e.preventDefault();
    emit("select", props.item.id, e.shiftKey ? "range" : "toggle");
  }
}

onBeforeUnmount(() => stopPointerTracking?.());
</script>

<template>
  <div
    class="item-row"
    :class="{ selected }"
    role="option"
    tabindex="0"
    :aria-selected="selected"
    @pointerdown="onPointerDown"
    @click="onClick"
    @dblclick="emit('open', item)"
    @keydown="onKeydown"
  >
    <div class="check" :class="{ on: selected }">
      <svg v-if="selected" width="9" height="9" viewBox="0 0 10 10" fill="none">
        <path d="M8 2.5 4.2 7.5 2 5.3" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round" />
      </svg>
    </div>
    <ThumbImg :kind="item.kind" :path="item.stagingPath" :ext="item.ext" :name="item.name" />
    <div class="item-body">
      <div class="item-name" :title="item.name">{{ item.name }}</div>
      <div class="item-meta">{{ meta }}</div>
    </div>
    <div class="row-actions">
      <button type="button" class="icon-btn" title="打开所在位置" aria-label="打开所在位置" @pointerdown.stop @click.stop="emit('reveal', item)">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
          <path d="M10 14 20 4M14 4h6v6M11 5H6a2 2 0 0 0-2 2v11a2 2 0 0 0 2 2h11a2 2 0 0 0 2-2v-5" />
        </svg>
      </button>
      <button type="button" class="icon-btn danger" title="移出暂存" aria-label="移出暂存" @pointerdown.stop @click.stop="emit('remove', item)">
        <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round">
          <path d="M5 7h14M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m3 0-1 13a1.5 1.5 0 0 1-1.5 1.4h-7A1.5 1.5 0 0 1 6.5 20L5.5 7" />
        </svg>
      </button>
    </div>
  </div>
</template>

<style scoped>
.item-row {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 10px;
  border-radius: var(--radius-card);
  cursor: default;
  position: relative;
  transition: background 130ms ease;
}
.item-row:hover {
  background: var(--surface-2);
}
.item-row:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}
.item-row.selected {
  background: var(--accent-soft);
}
.check {
  width: 16px;
  height: 16px;
  border-radius: 5px;
  border: 1.5px solid var(--line-strong);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  transition: all 140ms ease;
  opacity: 0;
}
.item-row:hover .check,
.item-row:focus-within .check,
.item-row.selected .check {
  opacity: 1;
}
.check.on {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--on-accent);
}
.item-body {
  flex: 1;
  min-width: 0;
}
.item-name {
  font-size: 13px;
  font-weight: 520;
  color: var(--ink);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.item-meta {
  font-size: 11px;
  color: var(--ink-3);
  letter-spacing: 0.01em;
  margin-top: 1px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.row-actions {
  display: flex;
  gap: 2px;
  opacity: 0;
  transition: opacity 130ms ease;
}
.item-row:hover .row-actions,
.item-row:focus-within .row-actions {
  opacity: 1;
}
.icon-btn {
  border: 0;
  background: transparent;
  width: 26px;
  height: 26px;
  border-radius: 7px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ink-2);
  cursor: pointer;
  transition: background 120ms ease, color 120ms ease;
}
.icon-btn:hover {
  background: var(--surface-3);
  color: var(--ink);
}
.icon-btn.danger:hover {
  background: color-mix(in oklab, var(--danger) 14%, transparent);
  color: var(--danger);
}
</style>
