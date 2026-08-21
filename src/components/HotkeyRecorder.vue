<script setup lang="ts">
/** 快捷键录入：点击后按下组合键捕获，Esc 取消 */
import { ref } from "vue";

const props = defineProps<{ modelValue: string; disabled?: boolean }>();
const emit = defineEmits<{ (e: "update:modelValue", v: string): void }>();

const recording = ref(false);

const KEY_LABELS: Record<string, string> = {
  Control: "Ctrl",
  Alt: "Alt",
  Shift: "Shift",
  Command: "Cmd",
  Meta: "Win",
  ArrowUp: "ArrowUp",
  ArrowDown: "ArrowDown",
  ArrowLeft: "ArrowLeft",
  ArrowRight: "ArrowRight",
  " ": "Space",
};

function toCombo(e: KeyboardEvent): string | null {
  if (!e.ctrlKey && !e.altKey && !e.metaKey) return null; // 必须带修饰键
  if (["Control", "Alt", "Shift", "Meta"].includes(e.key)) return null; // 只按了修饰键
  if (["Dead", "Process", "Unidentified"].includes(e.key)) return null;
  let main = e.key;
  if (main.length === 1) main = main.toUpperCase();
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  if (e.shiftKey) mods.push("Shift");
  if (e.metaKey) mods.push("Super");
  return [...mods, KEY_LABELS[main] ?? main].join("+");
}

function onKeydown(e: KeyboardEvent) {
  e.preventDefault();
  e.stopPropagation();
  if (e.key === "Escape") {
    recording.value = false;
    return;
  }
  const combo = toCombo(e);
  if (combo) {
    emit("update:modelValue", combo);
    recording.value = false;
  }
}
</script>

<template>
  <button
    type="button"
    class="hk"
    :class="{ recording, disabled }"
    :disabled="disabled"
    @click="recording = true"
    @keydown="recording && onKeydown($event)"
    @blur="recording = false"
  >
    <template v-if="recording">按下组合键…（Esc 取消）</template>
    <template v-else>{{ modelValue || "未设置" }}</template>
  </button>
</template>

<style scoped>
.hk {
  border: 1px solid var(--line-strong);
  background: var(--surface-raised);
  color: var(--ink);
  border-radius: 8px;
  padding: 6px 12px;
  font-size: 12.5px;
  font-family: inherit;
  cursor: pointer;
  min-width: 150px;
  text-align: center;
  transition: border-color 140ms ease, background 140ms ease;
}
.hk.recording {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-soft);
}
.hk.disabled {
  opacity: 0.5;
  cursor: default;
}
</style>
