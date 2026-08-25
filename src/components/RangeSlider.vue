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
  return Math.min(100, Math.max(0, ((props.value - props.min) / range) * 100));
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
  width: 190px;
  max-width: 100%;
  height: 28px;
  margin: 0;
  background: transparent;
  cursor: pointer;
  touch-action: none;
}
/* 轨道：更大的命中区域包住 6px 视觉轨道，填充段与空轨道保持清楚对比。 */
.range::-webkit-slider-runnable-track {
  height: 6px;
  border-radius: 999px;
  background: linear-gradient(
    to right,
    var(--accent) 0%,
    var(--accent) var(--fill),
    color-mix(in oklab, var(--line-strong) 72%, var(--surface-raised)) var(--fill),
    color-mix(in oklab, var(--line-strong) 72%, var(--surface-raised)) 100%
  );
  box-shadow: inset 0 0 0 1px color-mix(in oklab, var(--line-strong) 50%, transparent);
}
.range::-webkit-slider-thumb {
  -webkit-appearance: none;
  appearance: none;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: var(--surface-raised);
  border: 2px solid var(--surface-raised);
  box-shadow:
    0 0 0 1px var(--line-strong),
    0 2px 7px oklch(0.2 0.02 230 / 0.25);
  margin-top: -6px; /* (18 - 6) / 2，居中于轨道 */
  transition:
    transform 140ms var(--ease-out),
    border-color 140ms var(--ease-out),
    box-shadow 140ms var(--ease-out);
}
.range:hover::-webkit-slider-thumb {
  transform: scale(1.08);
  box-shadow:
    0 0 0 2px var(--accent),
    0 2px 8px oklch(0.2 0.02 230 / 0.28);
}
.range:active::-webkit-slider-thumb {
  transform: scale(1.12);
  box-shadow:
    0 0 0 5px var(--accent-soft),
    0 0 0 2px var(--accent),
    0 2px 8px oklch(0.2 0.02 230 / 0.28);
}
.range:focus-visible {
  outline: none;
}
.range:focus-visible::-webkit-slider-thumb {
  box-shadow:
    0 0 0 4px var(--accent-soft),
    0 0 0 2px var(--accent),
    0 2px 8px oklch(0.2 0.02 230 / 0.28);
}
.range:disabled {
  cursor: default;
  opacity: 0.45;
}
</style>
