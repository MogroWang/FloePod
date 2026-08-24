<script setup lang="ts">
/**
 * 自制滑块：细圆角轨道 + 强调色已填充段 + 圆形 thumb。
 * - 拖动中实时发出 update:value（1:1 跟随），松手后发出 commit（落库）
 * - hover 微放大、按下强调环，反馈落在 pointer-down（苹果式响应）
 * - 保留原生 input[type=range]，键盘方向键与焦点行为不变
 */
import { computed } from "vue";

const props = defineProps<{
  value: number;
  min: number;
  max: number;
  step: number;
  ariaLabel?: string;
}>();
const emit = defineEmits<{
  (e: "update:value", v: number): void;
  (e: "commit", v: number): void;
}>();

const percent = computed(() => {
  const range = props.max - props.min;
  if (range <= 0) return 0;
  return ((props.value - props.min) / range) * 100;
});

function onInput(e: Event) {
  emit("update:value", Number((e.target as HTMLInputElement).value));
}

function onChange(e: Event) {
  emit("commit", Number((e.target as HTMLInputElement).value));
}
</script>

<template>
  <input
    class="range"
    type="range"
    :min="min"
    :max="max"
    :step="step"
    :value="value"
    :aria-label="ariaLabel"
    :style="{ '--fill': percent + '%' }"
    @input="onInput"
    @change="onChange"
  />
</template>

<style scoped>
.range {
  -webkit-appearance: none;
  appearance: none;
  width: 150px;
  height: 18px;
  margin: 0;
  background: transparent;
  cursor: pointer;
}
/* 轨道：用渐变在 input 背景上画出「已填充段 + 未填充段」 */
.range::-webkit-slider-runnable-track {
  height: 4px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    var(--accent) 0%,
    var(--accent) var(--fill),
    var(--surface-3) var(--fill),
    var(--surface-3) 100%
  );
}
.range::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 15px;
  height: 15px;
  border-radius: 50%;
  background: var(--surface-raised);
  border: 1px solid var(--line-strong);
  box-shadow: 0 1px 4px oklch(0.2 0.02 230 / 0.22);
  margin-top: -5.5px; /* (15 - 4) / 2，居中于轨道 */
  transition:
    transform 140ms var(--ease-out),
    border-color 140ms var(--ease-out),
    box-shadow 140ms var(--ease-out);
}
.range:hover::-webkit-slider-thumb {
  transform: scale(1.12);
  border-color: var(--accent);
}
.range:active::-webkit-slider-thumb {
  transform: scale(1.18);
  border-color: var(--accent);
  box-shadow: 0 0 0 4px var(--accent-soft), 0 1px 4px oklch(0.2 0.02 230 / 0.22);
}
.range:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 3px;
}
.range:disabled {
  cursor: default;
  opacity: 0.45;
}
</style>
