<script setup lang="ts">
/** 导出冲突解决：覆盖 / 跳过 / 保留两者（自动重命名） */
import { computed } from "vue";
import { exportVerb } from "@/domain/exportPresentation";
import type { ConflictStrategy } from "@/domain/types";
import { previewSlice } from "@/lib/format";

const props = withDefaults(
  defineProps<{ names: string[]; mode: "copy" | "move"; busy?: boolean }>(),
  { busy: false },
);
const emit = defineEmits<{
  (e: "resolve", strategy: Exclude<ConflictStrategy, "ask">): void;
  (e: "cancel"): void;
}>();

const preview = computed(() => previewSlice(props.names));
const verb = computed(() => exportVerb(props.mode));
</script>

<template>
  <div class="conflict">
    <div class="title">{{ verb }}时目标位置已有同名文件</div>
    <ul class="names">
      <li v-for="(n, i) in preview.shown" :key="i">{{ n }}</li>
      <li v-if="preview.extra > 0" class="more">以及另外 {{ preview.extra }} 个</li>
    </ul>
    <div class="actions">
      <button
        type="button"
        class="act primary"
        :disabled="busy"
        @click="emit('resolve', 'rename')"
      >
        保留两者
      </button>
      <button type="button" class="act" :disabled="busy" @click="emit('resolve', 'overwrite')">
        覆盖
      </button>
      <button type="button" class="act ghost" :disabled="busy" @click="emit('resolve', 'skip')">
        跳过
      </button>
      <button type="button" class="act ghost" :disabled="busy" @click="emit('cancel')">
        取消
      </button>
    </div>
  </div>
</template>

<style scoped>
.conflict {
  padding: 18px 16px 14px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.title {
  font-size: 14.5px;
  font-weight: 650;
  letter-spacing: -0.01em;
}
.names {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 3px;
  max-height: 120px;
  overflow: auto;
}
.names li {
  font-size: 12px;
  color: var(--ink-2);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.names li.more {
  color: var(--ink-3);
}
.actions {
  display: grid;
  grid-template-columns: 1fr 1fr;
  gap: 8px;
}
</style>
