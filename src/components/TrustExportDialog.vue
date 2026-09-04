<script setup lang="ts">
import { onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { PrivacyScanResult } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { BROWSER_PREVIEW_EXPORT_ROOT } from "@/lib/env";

const props = defineProps<{ ids: number[]; podName: string }>();
const emit = defineEmits<{ (event: "close"): void; (event: "completed", message: string): void }>();

const scan = ref<PrivacyScanResult | null>(null);
const loading = ref(true);
const busy = ref(false);
const error = ref("");
const title = ref(`${props.podName}交接材料`);
const note = ref("");
const cleanMetadata = ref(true);

async function pickDestination(title: string): Promise<string | null> {
  if (!ipc.inTauri) return BROWSER_PREVIEW_EXPORT_ROOT;
  const value = await open({ directory: true, multiple: false, title });
  return typeof value === "string" ? value : null;
}

async function runScan() {
  loading.value = true;
  error.value = "";
  try {
    scan.value = await ipc.scanPrivacy(props.ids);
  } catch (reason) {
    error.value = `隐私检查失败：${String(reason)}`;
  } finally {
    loading.value = false;
  }
}

async function safeExport() {
  if (busy.value) return;
  const destination = await pickDestination("选择清理后副本的保存位置");
  if (!destination) return;
  busy.value = true;
  try {
    const result = await ipc.safeExportItems(props.ids, destination);
    emit(
      "completed",
      `已生成 ${result.completed.length} 个清理副本${result.failed.length ? `，${result.failed.length} 项失败` : ""}`,
    );
  } catch (reason) {
    error.value = `安全导出失败：${String(reason)}`;
  } finally {
    busy.value = false;
  }
}

async function createHandoff() {
  if (busy.value || !title.value.trim()) return;
  const destination = await pickDestination("选择交接包的保存位置");
  if (!destination) return;
  busy.value = true;
  try {
    const result = await ipc.createHandoff(
      props.ids,
      destination,
      title.value.trim(),
      note.value.trim(),
      cleanMetadata.value,
    );
    emit(
      "completed",
      `交接包已生成：${result.files.length} 个文件${result.missing.length ? `，${result.missing.length} 项需检查` : ""}`,
    );
  } catch (reason) {
    error.value = `交接包生成失败：${String(reason)}`;
  } finally {
    busy.value = false;
  }
}

onMounted(runScan);
</script>

<template>
  <div class="trust-dialog" role="dialog" aria-modal="true" aria-labelledby="trust-title">
    <header>
      <div>
        <h2 id="trust-title">安全导出与可信交接</h2>
        <p>所有检查和清理均在本机完成，原文件不会被修改。</p>
      </div>
      <button type="button" class="icon-close" aria-label="关闭" :disabled="busy" @click="emit('close')">×</button>
    </header>

    <p v-if="loading" class="notice" role="status">正在本地检查 {{ ids.length }} 个项目…</p>
    <template v-else-if="scan">
      <div class="scan-summary" role="status">
        已检查 {{ scan.filesScanned }} 个文件，发现 {{ scan.issues.length }} 个可能的隐私或交付问题。
      </div>
      <ul v-if="scan.issues.length" class="issue-list">
        <li v-for="(issue, index) in scan.issues" :key="`${issue.path}-${issue.code}-${index}`" :data-severity="issue.severity">
          <div>
            <strong>{{ issue.message }}</strong>
            <small>{{ issue.path }}</small>
          </div>
          <span>{{ issue.canClean ? "可生成清理副本" : "请人工确认" }}</span>
        </li>
      </ul>
      <p v-else class="notice success">没有发现已知的常见隐私问题，仍建议交付前人工复核。</p>
      <p class="disclaimer">{{ scan.disclaimer }}</p>
    </template>

    <p v-if="error" class="error" role="alert">{{ error }}</p>

    <section class="action-section">
      <div>
        <h3>生成清理后的副本</h3>
        <p>移除图片 EXIF/GPS、PDF 文档属性以及 Office 作者和公司字段；不修改原件。</p>
      </div>
      <button type="button" class="button" :disabled="busy || loading" @click="safeExport">选择位置并生成</button>
    </section>

    <section class="handoff-section">
      <h3>生成可信交接包</h3>
      <label>
        <span>交接包名称</span>
        <input v-model="title" maxlength="80" />
      </label>
      <label>
        <span>交接说明</span>
        <textarea v-model="note" rows="3" placeholder="用途、缺失材料、保存期限或接收人注意事项" />
      </label>
      <label class="check-label">
        <input v-model="cleanMetadata" type="checkbox" />
        <span>逐文件生成元数据清理副本后再打包</span>
      </label>
      <button type="button" class="button primary" :disabled="busy || loading || !title.trim()" @click="createHandoff">
        {{ busy ? "正在处理…" : "选择位置并生成交接包" }}
      </button>
      <p>交接包包含文件清单 CSV、SHA256SUMS、JSON 机器清单和可直接打开的交接说明 HTML。</p>
    </section>
  </div>
</template>

<style scoped>
.trust-dialog {
  display: grid;
  gap: 13px;
  padding: 16px;
}
header,
.action-section {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
h2,
h3,
p {
  margin: 0;
}
h2 {
  font-size: 16px;
}
h3 {
  font-size: 13.5px;
}
header p,
.action-section p,
.handoff-section > p,
.disclaimer {
  margin-top: 3px;
  color: var(--ink-2);
  font-size: 11.5px;
}
.icon-close {
  width: 32px;
  height: 32px;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--ink-2);
  font-size: 22px;
}
.notice,
.scan-summary,
.error {
  padding: 9px 11px;
  border-radius: 9px;
  background: var(--surface-2);
}
.success {
  color: var(--ink-2);
}
.error {
  color: var(--danger);
}
.issue-list {
  display: grid;
  gap: 6px;
  max-height: 170px;
  margin: 0;
  padding: 0;
  overflow: auto;
  list-style: none;
}
.issue-list li {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 9px;
  padding: 8px 9px;
  border-left: 3px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
}
.issue-list li[data-severity="high"] {
  border-left-color: var(--danger);
}
.issue-list li strong,
.issue-list li small {
  display: block;
}
.issue-list li small {
  margin-top: 2px;
  color: var(--ink-3);
  overflow-wrap: anywhere;
}
.issue-list li > span {
  flex-shrink: 0;
  color: var(--ink-2);
  font-size: 10.5px;
}
.action-section,
.handoff-section {
  padding: 11px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-raised);
}
.handoff-section {
  display: grid;
  gap: 9px;
}
.handoff-section label:not(.check-label) {
  display: grid;
  gap: 4px;
  color: var(--ink-2);
  font-size: 11.5px;
}
input,
textarea {
  width: 100%;
  padding: 7px 9px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface);
  color: var(--ink);
  resize: vertical;
}
.check-label {
  display: flex;
  align-items: center;
  gap: 8px;
  color: var(--ink-2);
  font-size: 11.5px;
}
.check-label input {
  width: auto;
}
.button {
  min-height: 34px;
  padding: 0 12px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
  color: var(--ink);
  white-space: nowrap;
}
.button.primary {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--on-accent);
}
</style>
