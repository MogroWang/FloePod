<script setup lang="ts">
import { onMounted, ref, watch } from "vue";
import type { PodSecurity } from "@/domain/types";
import { ipc } from "@/ipc/client";
import SettingsRow from "./SettingsRow.vue";
import ToggleSwitch from "./ToggleSwitch.vue";

const props = defineProps<{
  podId: number;
  folder: string;
  security: PodSecurity;
}>();
const emit = defineEmits<{ (event: "update", value: PodSecurity): void }>();

const status = ref<{ locked: boolean; efsEncrypted: boolean; expiresSoon: number } | null>(null);
const message = ref("");

function update(patch: Partial<PodSecurity>) {
  emit("update", { ...props.security, ...patch });
}

async function refreshStatus() {
  try {
    status.value = await ipc.getPodSecurityStatus(props.podId);
    message.value = status.value.efsEncrypted
      ? "Windows EFS 已保护此匣目录。"
      : props.security.enabled
        ? "尚未确认 EFS 加密，请检查该目录是否位于支持 EFS 的 NTFS 卷。"
        : "敏感匣未启用。";
  } catch (error) {
    message.value = String(error);
  }
}

async function lockNow() {
  await ipc.lockSensitivePod(props.podId);
  await refreshStatus();
}

async function unlockNow() {
  try {
    status.value = await ipc.unlockSensitivePod(props.podId);
    message.value = "已通过 Windows Hello 解锁。";
  } catch (error) {
    message.value = `解锁失败：${String(error)}`;
  }
}

watch(() => props.security.enabled, refreshStatus);
onMounted(refreshStatus);
</script>

<template>
  <div class="security-editor">
    <div class="security-note">
      <strong>不自制加密算法</strong>
      <p>文件静态加密交给 Windows EFS，应用解锁交给 Windows Hello/PIN。启用失败时不会保存成“假加密”状态。</p>
    </div>
    <SettingsRow label="启用敏感匣" hint="要求暂存目录位于支持 EFS 的 NTFS 卷">
      <ToggleSwitch
        label="启用敏感匣"
        :model-value="security.enabled"
        :disabled="!folder"
        @update:model-value="(value) => update({ enabled: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="使用 Windows Hello 解锁" hint="不在 FloePod 中保存密码或恢复密钥">
      <ToggleSwitch
        label="使用 Windows Hello 解锁"
        :model-value="security.requireWindowsHello"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ requireWindowsHello: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="自动锁定" hint="0 表示仅在退出或手动锁定时重新锁定">
      <div class="number-control">
        <input
          type="number"
          min="0"
          max="1440"
          :value="security.autoLockMinutes"
          :disabled="!security.enabled"
          aria-label="自动锁定分钟数"
          @change="update({ autoLockMinutes: Number(($event.target as HTMLInputElement).value) })"
        />
        <span>分钟</span>
      </div>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="保留期限" hint="0 表示不自动清理；到期文件移入系统回收站">
      <div class="number-control">
        <input
          type="number"
          min="0"
          max="3650"
          :value="security.retentionDays"
          :disabled="!security.enabled"
          aria-label="保留期限天数"
          @change="update({ retentionDays: Number(($event.target as HTMLInputElement).value) })"
        />
        <span>天</span>
      </div>
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="导出后清理暂存副本" hint="成功导出后移出匣，操作仍进入时间线">
      <ToggleSwitch
        label="导出后清理暂存副本"
        :model-value="security.cleanupAfterExport"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ cleanupAfterExport: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="禁止缩略图" hint="避免敏感图片预览进入 WebView 内存">
      <ToggleSwitch
        label="禁止缩略图"
        :model-value="security.suppressThumbnails"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ suppressThumbnails: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="禁止全文索引" hint="文件名仍保留在匣中，正文和 OCR 不写入索引">
      <ToggleSwitch
        label="禁止全文索引"
        :model-value="security.suppressIndex"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ suppressIndex: value })"
      />
    </SettingsRow>
    <div class="security-status" role="status" aria-live="polite">
      <span>{{ message }}</span>
      <span v-if="status?.expiresSoon">{{ status.expiresSoon }} 项已达到提醒或清理期限。</span>
      <div v-if="security.enabled" class="security-actions">
        <button type="button" @click="refreshStatus">检查状态</button>
        <button v-if="status?.locked" type="button" class="primary" @click="unlockNow">解锁</button>
        <button v-else type="button" @click="lockNow">立即锁定</button>
      </div>
    </div>
    <p class="disable-note">关闭敏感匣只停止应用内锁定，不会擅自解密磁盘上已有文件；如需解密请使用 Windows 文件属性。</p>
  </div>
</template>

<style scoped>
.security-editor {
  overflow: hidden;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
}
.security-editor :deep(.row) {
  padding: 12px 14px;
}
.security-note,
.security-status {
  display: grid;
  gap: 4px;
  margin: 12px;
  padding: 10px;
  border-radius: 9px;
  background: var(--accent-soft);
}
.security-note p,
.disable-note {
  margin: 0;
  color: var(--ink-2);
  font-size: 11px;
}
.disable-note {
  padding: 0 14px 13px;
}
.sep {
  height: 1px;
  margin: 0 14px;
  background: var(--line);
}
.number-control,
.security-actions {
  display: flex;
  align-items: center;
  gap: 7px;
}
.number-control input {
  width: 92px;
  min-height: 34px;
  padding: 5px 8px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface);
  color: var(--ink);
}
.number-control span,
.security-status {
  color: var(--ink-2);
  font-size: 11.5px;
}
.security-actions {
  margin-top: 5px;
}
button {
  min-height: 32px;
  padding: 0 10px;
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  background: var(--surface-raised);
  color: var(--ink);
}
button.primary {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--on-accent);
}
</style>
