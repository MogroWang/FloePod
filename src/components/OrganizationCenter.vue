<script setup lang="ts">
import { onMounted, ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { PolicyStatus } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { BROWSER_PREVIEW_EXPORT_ROOT } from "@/lib/env";
import { useSettingsStore } from "@/stores/settings";

const settingsStore = useSettingsStore();
const policy = ref<PolicyStatus | null>(null);
const message = ref("");
const busy = ref(false);

async function chooseDirectory(title: string): Promise<string | null> {
  if (!ipc.inTauri) return BROWSER_PREVIEW_EXPORT_ROOT;
  const value = await open({ directory: true, multiple: false, title });
  return typeof value === "string" ? value : null;
}

async function loadPolicy() {
  try {
    policy.value = await ipc.getOrganizationPolicy();
  } catch (error) {
    message.value = `读取机构策略失败：${String(error)}`;
  }
}

async function exportAudit(format: "json" | "csv") {
  const directory = await chooseDirectory("选择审计记录保存位置");
  if (!directory) return;
  busy.value = true;
  try {
    const result = await ipc.exportAuditLog(directory, format);
    message.value = `已导出 ${result.records} 条本地审计记录：${result.path}`;
  } catch (error) {
    message.value = `审计导出失败：${String(error)}`;
  } finally {
    busy.value = false;
  }
}

async function exportDiagnostics() {
  const directory = await chooseDirectory("选择诊断包保存位置");
  if (!directory) return;
  busy.value = true;
  try {
    const result = await ipc.exportDiagnosticBundle(directory);
    message.value = `本地诊断包已生成：${result.path}`;
  } catch (error) {
    message.value = `诊断包生成失败：${String(error)}`;
  } finally {
    busy.value = false;
  }
}

async function exportSettings() {
  const directory = await chooseDirectory("选择设置备份位置");
  if (!directory) return;
  busy.value = true;
  try {
    const result = await ipc.exportSettingsFile(directory);
    message.value = `设置备份已生成：${result.path}`;
  } catch (error) {
    message.value = `设置导出失败：${String(error)}`;
  } finally {
    busy.value = false;
  }
}

async function importSettings() {
  if (!ipc.inTauri) {
    message.value = "浏览器预览已模拟导入设置。";
    return;
  }
  const source = await open({
    directory: false,
    multiple: false,
    filters: [{ name: "FloePod 设置", extensions: ["json"] }],
    title: "选择 FloePod 设置备份",
  });
  if (typeof source !== "string") return;
  busy.value = true;
  try {
    await ipc.importSettingsFile(source);
    await settingsStore.load();
    message.value = "设置已验证并导入。";
  } catch (error) {
    message.value = `设置导入失败，原设置未改变：${String(error)}`;
  } finally {
    busy.value = false;
  }
}

onMounted(loadPolicy);
</script>

<template>
  <div class="organization-center">
    <div v-if="policy" class="policy-summary" :data-managed="policy.managed">
      <strong>{{ policy.managed ? policy.policy.organizationName || "受机构策略管理" : "个人 / 社区模式" }}</strong>
      <span v-if="policy.managed">策略文件：{{ policy.source }}</span>
      <span v-else>没有安装机构策略文件，全部数据和设置由本机用户控制。</span>
      <ul v-if="policy.managed">
        <li v-if="policy.policy.disableMove">禁止移动源文件</li>
        <li v-if="policy.policy.requirePrivacyScan">普通导出前必须进行隐私检查</li>
        <li v-if="policy.policy.lockRules">规则模板由管理员锁定</li>
        <li v-if="policy.policy.disableFulltextIndex">禁止全文和 OCR 索引</li>
        <li v-if="policy.policy.mandatoryRetentionDays">最长保留 {{ policy.policy.mandatoryRetentionDays }} 天</li>
      </ul>
    </div>

    <div class="organization-actions">
      <button type="button" :disabled="busy" @click="exportAudit('json')">导出审计 JSON</button>
      <button type="button" :disabled="busy" @click="exportAudit('csv')">导出审计 CSV</button>
      <button type="button" :disabled="busy" @click="exportDiagnostics">生成脱敏诊断包</button>
      <button type="button" :disabled="busy" @click="exportSettings">备份设置</button>
      <button type="button" :disabled="busy" @click="importSettings">导入设置</button>
    </div>
    <p class="privacy-note">诊断包默认隐藏暂存路径，不包含文件内容、缩略图、OCR 正文或文件名；只有机构策略明确允许时才保留路径。</p>
    <p class="message" role="status" aria-live="polite">{{ message }}</p>
  </div>
</template>

<style scoped>
/* 卡片规格与设置窗口 .settings-card 保持一致：同圆角、同内边距、同控件规格 */
.organization-center {
  display: grid;
  gap: 10px;
  padding: 14px 16px 16px;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
}
.policy-summary {
  display: grid;
  gap: 4px;
  padding: 12px;
  border: 1px solid var(--line);
  border-radius: 10px;
  background: var(--surface-raised);
}
.policy-summary[data-managed="true"] {
  border-color: color-mix(in srgb, var(--accent) 50%, var(--line));
}
.policy-summary span,
.policy-summary li,
.privacy-note,
.message {
  color: var(--ink-2);
  font-size: 11.5px;
}
.policy-summary ul {
  margin: 5px 0 0;
  padding-left: 18px;
}
.organization-actions {
  display: flex;
  flex-wrap: wrap;
  gap: 7px;
}
button {
  min-height: 32px;
  padding: 6px 13px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
  color: var(--ink);
  font-size: 12.5px;
  font-weight: 550;
  font-family: inherit;
  cursor: pointer;
  transition: background 150ms var(--ease-out), border-color 150ms var(--ease-out);
}
button:hover {
  background: var(--surface-hover);
}
button:disabled {
  opacity: 0.55;
  cursor: wait;
}
.privacy-note,
.message {
  min-height: 18px;
  margin: 0;
}
</style>
