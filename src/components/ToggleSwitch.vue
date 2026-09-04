<script setup lang="ts">
/** 开关：按下即反馈，状态变化有弹性 */
const props = defineProps<{ modelValue: boolean; label: string; disabled?: boolean }>();
const emit = defineEmits<{ (e: "update:modelValue", v: boolean): void }>();

function toggle() {
  if (!props.disabled) emit("update:modelValue", !props.modelValue);
}
</script>

<template>
  <button
    type="button"
    role="switch"
    :aria-checked="modelValue"
    :aria-label="label"
    :disabled="disabled"
    class="switch"
    :class="{ on: modelValue }"
    @click="toggle"
  >
    <span class="knob" />
  </button>
</template>

<style scoped>
.switch {
  width: 40px;
  height: 24px;
  border-radius: 999px;
  border: 0;
  padding: 2px;
  background: var(--surface-3);
  box-shadow: inset 0 0 0 1px var(--line-strong);
  cursor: pointer;
  transition: background 200ms ease, box-shadow 200ms ease;
}
.switch.on {
  background: var(--accent);
  box-shadow: none;
}
.switch:disabled {
  opacity: 0.45;
  cursor: default;
}
.knob {
  display: block;
  width: 20px;
  height: 20px;
  border-radius: 999px;
  background: #fff;
  box-shadow: 0 1px 3px oklch(0 0 0 / 0.25);
  transition: transform 200ms var(--ease-out);
}
.switch.on .knob {
  transform: translateX(16px);
}
</style>
