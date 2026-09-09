<script setup lang="ts">
import { computed } from "vue";
import type { PrivacyScanResult } from "@/domain/types";
import { privacySummary } from "@/domain/privacy";
const props = defineProps<{ scan: PrivacyScanResult }>();
const summary = computed(() => privacySummary(props.scan));
</script>

<template>
  <div class="scan-summary" role="status">
    已收集 {{ scan.filesScanned }} 个文件；元数据检查完成 {{ summary.checked }} 项， 跳过
    {{ summary.skipped }} 项，失败 {{ summary.failed }} 项。
  </div>
  <p v-if="summary.incomplete" class="incomplete" role="status">
    部分项目未完成元数据检查，不能据此判断没有隐私风险。
  </p>
  <ul v-if="summary.pending.length" class="pending-list">
    <li v-for="(file, index) in summary.pending" :key="`${file.path}-${index}`">
      <strong>{{ file.status === "failed" ? "检查失败" : "未检查" }}</strong
      >：{{ file.reason }}
      <small>{{ file.path }}</small>
    </li>
  </ul>
  <p v-if="summary.canReportNoKnownIssues" class="notice">
    已完成的检查未发现已知常见隐私问题，仍建议交付前复核。
  </p>
</template>

<style scoped>
.scan-summary,
.incomplete,
.notice {
  margin: 0;
  padding: 9px 11px;
  border-radius: 9px;
  background: var(--surface-2);
}
.pending-list {
  margin: 0;
  padding: 0 0 0 20px;
  max-height: 160px;
  overflow: auto;
}
small {
  display: block;
  color: var(--ink-3);
  overflow-wrap: anywhere;
}
</style>
