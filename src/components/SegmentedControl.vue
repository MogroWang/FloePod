<script setup lang="ts">
/**
 * 分段选择（iOS 风格）：当前段平滑滑动。
 * 缩略图按活动按钮的实际位置与宽度像素级对齐（translateX 百分比相对自身宽度，
 * 按钮文案长短不一，无法用等分百分比对齐）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";

const props = defineProps<{
  options: { value: string; label: string }[];
  modelValue: string;
  disabled?: boolean;
}>();
const emit = defineEmits<{ (e: "update:modelValue", v: string): void }>();

const segRef = ref<HTMLElement | null>(null);
const thumbStyle = ref<{ width: string; transform: string }>({
  width: "0px",
  transform: "translateX(0px)",
});

const index = computed(() =>
  Math.max(0, props.options.findIndex((o) => o.value === props.modelValue)),
);

let ro: ResizeObserver | null = null;

function positionThumb() {
  const seg = segRef.value;
  if (!seg) return;
  const btn = seg.querySelector<HTMLElement>(".seg-item.active");
  if (!btn) return;
  thumbStyle.value = {
    width: `${btn.offsetWidth}px`,
    transform: `translateX(${btn.offsetLeft}px)`,
  };
}

async function onOptionKeydown(e: KeyboardEvent, value: string) {
  if (props.disabled) return;
  const current = props.options.findIndex((option) => option.value === value);
  if (current < 0 || props.options.length === 0) return;
  let next = current;
  if (e.key === "ArrowLeft" || e.key === "ArrowUp") {
    next = (current - 1 + props.options.length) % props.options.length;
  } else if (e.key === "ArrowRight" || e.key === "ArrowDown") {
    next = (current + 1) % props.options.length;
  } else if (e.key === "Home") {
    next = 0;
  } else if (e.key === "End") {
    next = props.options.length - 1;
  } else {
    return;
  }
  e.preventDefault();
  emit("update:modelValue", props.options[next].value);
  await nextTick();
  segRef.value?.querySelectorAll<HTMLButtonElement>(".seg-item")[next]?.focus();
}

onMounted(() => {
  positionThumb();
  ro = new ResizeObserver(() => positionThumb());
  if (segRef.value) ro.observe(segRef.value);
});
onBeforeUnmount(() => ro?.disconnect());

watch([index, () => props.options], async () => {
  await nextTick();
  positionThumb();
});
</script>

<template>
  <div ref="segRef" class="seg" :class="{ disabled }" role="radiogroup" :aria-disabled="disabled">
    <div class="seg-thumb" :style="thumbStyle" />
    <button
      v-for="o in options"
      :key="o.value"
      type="button"
      role="radio"
      :aria-checked="o.value === modelValue"
      :tabindex="o.value === modelValue ? 0 : -1"
      :disabled="disabled"
      class="seg-item"
      :class="{ active: o.value === modelValue }"
      @click="!disabled && emit('update:modelValue', o.value)"
      @keydown="onOptionKeydown($event, o.value)"
    >
      {{ o.label }}
    </button>
  </div>
</template>

<style scoped>
.seg {
  position: relative;
  display: inline-flex;
  padding: 2px;
  background: var(--surface-2);
  border-radius: 9px;
  width: fit-content;
}
.seg-thumb {
  position: absolute;
  top: 2px;
  left: 0;
  height: calc(100% - 4px);
  background: var(--surface-raised);
  border-radius: 7px;
  box-shadow: 0 1px 4px oklch(0.2 0.02 230 / 0.18);
  transition: transform 180ms var(--ease-out), width 180ms var(--ease-out);
}
.seg-item {
  position: relative;
  z-index: 1;
  border: 0;
  background: transparent;
  padding: 4px 12px;
  font-size: 12.5px;
  color: var(--ink-2);
  cursor: pointer;
  border-radius: 7px;
  transition: color 160ms ease;
  font-family: inherit;
  white-space: nowrap;
}
.seg-item.active {
  color: var(--ink);
  font-weight: 550;
}
.seg.disabled {
  opacity: 0.58;
}
.seg-item:disabled {
  cursor: not-allowed;
}
</style>
