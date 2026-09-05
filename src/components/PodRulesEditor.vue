<script setup lang="ts">
import { computed } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import type { PodRules } from "@/domain/types";
import { ipc } from "@/ipc/client";
import SettingsRow from "./SettingsRow.vue";
import ToggleSwitch from "./ToggleSwitch.vue";

const props = defineProps<{ rules: PodRules }>();
const emit = defineEmits<{ (event: "update", value: PodRules): void }>();

const extensionText = computed(() => props.rules.allowedExtensions.join(", "));

const templates: Array<{ value: string; label: string; patch: Partial<PodRules> }> = [
  { value: "manual", label: "自定义", patch: {} },
  {
    value: "expense",
    label: "待报销",
    patch: {
      allowedExtensions: ["pdf", "jpg", "jpeg", "png"],
      renamePattern: "{date}_{stem}",
      subfolderPattern: "{year}-{month}",
      duplicatePolicy: "reject",
      checksumSidecar: true,
    },
  },
  {
    value: "delivery",
    label: "客户交付",
    patch: {
      renamePattern: "{date}_{name}",
      duplicatePolicy: "reject",
      checksumSidecar: true,
    },
  },
  {
    value: "homework",
    label: "学生作业",
    patch: {
      renamePattern: "{date}_{stem}",
      subfolderPattern: "{year}/{month}",
      duplicatePolicy: "reject",
    },
  },
  {
    value: "meeting",
    label: "会议资料",
    patch: { subfolderPattern: "{year}/{month}", expireDays: 30 },
  },
  {
    value: "photos",
    label: "照片整理",
    patch: {
      allowedExtensions: ["jpg", "jpeg", "png", "webp", "gif", "bmp"],
      subfolderPattern: "{year}/{month}",
      duplicatePolicy: "reject",
    },
  },
  {
    value: "contracts",
    label: "合同待签",
    patch: { allowedExtensions: ["pdf"], duplicatePolicy: "reject", checksumSidecar: true },
  },
  {
    value: "scans",
    label: "扫描件",
    patch: {
      allowedExtensions: ["pdf", "jpg", "jpeg", "png"],
      renamePattern: "扫描_{date}_{stem}",
      subfolderPattern: "{year}-{month}",
    },
  },
  {
    value: "downloads",
    label: "下载待整理",
    patch: { sourceFolder: "", expireDays: 7, duplicatePolicy: "reject" },
  },
];

function update(patch: Partial<PodRules>) {
  emit("update", { ...props.rules, ...patch });
}

function applyTemplate(value: string) {
  const template = templates.find((item) => item.value === value);
  if (!template) return;
  update({
    template: value,
    enabled: value === "manual" ? props.rules.enabled : true,
    ...template.patch,
  });
}

function updateExtensions(value: string) {
  update({
    allowedExtensions: [...new Set(
      value
        .split(/[，,;；\s]+/)
        .map((extension) => extension.trim().replace(/^\./, "").toLowerCase())
        .filter(Boolean),
    )],
    template: "manual",
  });
}

async function pickSourceFolder() {
  if (!ipc.inTauri) {
    update({ sourceFolder: "D:\\Downloads", template: "manual" });
    return;
  }
  const folder = await open({ directory: true, multiple: false, title: "选择允许的来源文件夹" });
  if (typeof folder === "string") update({ sourceFolder: folder, template: "manual" });
}
</script>

<template>
  <div class="rules-editor">
    <SettingsRow label="启用规则" hint="只运行下方可见规则，不执行脚本或联网动作">
      <ToggleSwitch label="启用规则" :model-value="rules.enabled" @update:model-value="(value) => update({ enabled: value })" />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="工作流模板" hint="选择后仍可继续修改每一项">
      <select class="rule-input compact" :value="rules.template" aria-label="工作流模板" @change="applyTemplate(($event.target as HTMLSelectElement).value)">
        <option v-for="template in templates" :key="template.value" :value="template.value">
          {{ template.label }}
        </option>
      </select>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="允许的文件类型" hint="留空表示全部；例如 pdf, jpg, docx">
      <input
        class="rule-input"
        :value="extensionText"
        :disabled="!rules.enabled"
        aria-label="允许的文件扩展名"
        @change="updateExtensions(($event.target as HTMLInputElement).value)"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="文件名必须包含" hint="不区分大小写；留空不限制">
      <input
        class="rule-input"
        :value="rules.nameContains"
        :disabled="!rules.enabled"
        maxlength="128"
        aria-label="文件名必须包含"
        @change="update({ nameContains: ($event.target as HTMLInputElement).value, template: 'manual' })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="指定来源文件夹" hint="留空不限制来源">
      <div class="path-control">
        <input class="rule-input path" :value="rules.sourceFolder" aria-label="指定来源文件夹" readonly />
        <button type="button" class="rule-button" :disabled="!rules.enabled" @click="pickSourceFolder">选择…</button>
        <button v-if="rules.sourceFolder" type="button" class="rule-button" @click="update({ sourceFolder: '', template: 'manual' })">清除</button>
      </div>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="单个文件上限" hint="0 表示不限制，最大 102400 MB">
      <div class="number-control">
        <input
          class="rule-input number"
          type="number"
          min="0"
          max="102400"
          :value="rules.maxSizeMb"
          :disabled="!rules.enabled"
          aria-label="单个文件上限（MB）"
          @change="update({ maxSizeMb: Number(($event.target as HTMLInputElement).value), template: 'manual' })"
        />
        <span>MB</span>
      </div>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="自动命名" hint="令牌：{name} {stem} {ext} {date} {year} {month} {day}">
      <input
        class="rule-input mono"
        :value="rules.renamePattern"
        :disabled="!rules.enabled"
        aria-label="自动命名规则"
        @change="update({ renamePattern: ($event.target as HTMLInputElement).value, template: 'manual' })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="自动放入子目录" hint="例如 {year}/{month}；留空直接放入匣目录">
      <input
        class="rule-input mono"
        :value="rules.subfolderPattern"
        :disabled="!rules.enabled"
        aria-label="自动子目录规则"
        @change="update({ subfolderPattern: ($event.target as HTMLInputElement).value, template: 'manual' })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="拒绝内容重复文件" hint="比较大小和 SHA-256，不只比较名称">
      <ToggleSwitch
        label="拒绝内容重复文件"
        :model-value="rules.duplicatePolicy === 'reject'"
        :disabled="!rules.enabled"
        @update:model-value="(value) => update({ duplicatePolicy: value ? 'reject' : 'allow', template: 'manual' })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="生成 SHA-256 校验文件" hint="文件暂存后在旁边生成 .sha256">
      <ToggleSwitch
        label="生成 SHA-256 校验文件"
        :model-value="rules.checksumSidecar"
        :disabled="!rules.enabled"
        @update:model-value="(value) => update({ checksumSidecar: value, template: 'manual' })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="到期提醒" hint="0 表示不提醒；到期清理由敏感匣设置单独控制">
      <div class="number-control">
        <input
          class="rule-input number"
          type="number"
          min="0"
          max="3650"
          :value="rules.expireDays"
          :disabled="!rules.enabled"
          aria-label="到期提醒天数"
          @change="update({ expireDays: Number(($event.target as HTMLInputElement).value), template: 'manual' })"
        />
        <span>天</span>
      </div>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="导出后移出暂存" hint="成功交付后自动移入 24 小时可撤销区">
      <ToggleSwitch
        label="导出后移出暂存"
        :model-value="rules.removeAfterExport"
        :disabled="!rules.enabled"
        @update:model-value="(value) => update({ removeAfterExport: value, template: 'manual' })"
      />
    </SettingsRow>
  </div>
</template>

<style scoped>
/* 卡片规格与设置窗口 .settings-card 保持一致：同圆角、同行距、同分隔线 */
.rules-editor {
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
}
.rules-editor :deep(.row) {
  padding: 14px 16px;
}
.sep {
  height: 1px;
  margin: 0 16px;
  background: var(--line);
}
.rule-input {
  width: 210px;
  min-height: 32px;
  padding: 6px 10px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
  color: var(--ink);
  font-size: 12.5px;
  font-family: inherit;
  outline: none;
}
.rule-input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.rule-input.compact {
  width: 150px;
}
.rule-input.number {
  width: 92px;
}
.rule-input.path {
  min-width: 0;
  flex: 1;
}
.mono {
  font-family: ui-monospace, "Cascadia Code", monospace;
  font-size: 11px;
}
.path-control,
.number-control {
  display: flex;
  align-items: center;
  gap: 7px;
}
.path-control {
  width: min(330px, 45vw);
}
.number-control span {
  color: var(--ink-2);
  font-size: 12px;
}
.rule-button {
  flex-shrink: 0;
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
.rule-button:hover {
  background: var(--surface-hover);
}
</style>
