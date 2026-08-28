<script setup lang="ts">
/** 拖入动作询问：复制 / 移动 / 创建快捷方式 / 取消 + 记住选择 */
import { computed } from "vue";
import type { DropAction } from "@/domain/types";
import { previewSlice } from "@/lib/format";

const props = withDefaults(defineProps<{ paths: string[]; busy?: boolean }>(), {
  busy: false,
});
const emit = defineEmits<{
  (e: "choose", action: DropAction, remember: boolean): void;
  (e: "cancel"): void;
}>();

const remember = defineModel<boolean>("remember", { default: false });

const names = computed(() => props.paths.map((p) => p.split(/[\\/]/).pop() ?? p));
const preview = computed(() => previewSlice(names.value));
</script>

<template>
  <div class="chooser">
    <div class="chooser-title">暂存 {{ paths.length }} 项</div>
    <ul class="chooser-list">
      <li v-for="(n, i) in preview.shown" :key="i" :title="paths[i]">{{ n }}</li>
      <li v-if="preview.extra > 0" class="more">以及另外 {{ preview.extra }} 项</li>
    </ul>

    <div class="chooser-actions">
      <button
        type="button"
        class="act primary"
        :disabled="busy"
        @click="emit('choose', 'copy', remember)"
      >
        复制
      </button>
      <button type="button" class="act" :disabled="busy" @click="emit('choose', 'move', remember)">
        移动
      </button>
      <button
        type="button"
        class="act"
        :disabled="busy"
        @click="emit('choose', 'shortcut', remember)"
      >
        创建快捷方式
      </button>
      <button type="button" class="act ghost" :disabled="busy" @click="emit('cancel')">取消</button>
    </div>

    <label class="remember">
      <input v-model="remember" type="checkbox" :disabled="busy" />
      <span>记住选择，之后不再询问（可在设置中改回）</span>
    </label>
  </div>
</template>

<style scoped>
.chooser {
  padding: 18px 16px 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.chooser-title {
  font-size: 15px;
  font-weight: 650;
  letter-spacing: -0.01em;
  color: var(--ink);
}
.chooser-list {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 132px;
  overflow: auto;
}
.chooser-list li {
  font-size: 12px;
  color: var(--ink-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.chooser-list li.more {
  color: var(--ink-3);
}
.chooser-actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}
.remember {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: 12px;
  color: var(--ink-2);
  cursor: pointer;
}
.remember input {
  accent-color: var(--accent);
}
</style>
