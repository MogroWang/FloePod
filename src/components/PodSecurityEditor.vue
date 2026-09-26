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
      ? "已由 Windows EFS 保护。"
      : props.security.enabled
        ? "尚未确认 EFS 加密；暂存目录需位于支持 EFS 的 NTFS 卷。"
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
    <SettingsRow label="启用敏感匣" hint="暂存目录需位于支持 EFS 的 NTFS 卷">
      <ToggleSwitch
        label="启用敏感匣"
        :model-value="security.enabled"
        :disabled="!folder"
        @update:model-value="(value) => update({ enabled: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="使用 Windows Hello 解锁" hint="密码与恢复密钥不经过 FloePod">
      <ToggleSwitch
        label="使用 Windows Hello 解锁"
        :model-value="security.requireWindowsHello"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ requireWindowsHello: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="自动锁定" hint="0 表示仅退出或手动锁定时">
      <div class="number-control">
        <input
          class="rule-input number"
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
    <SettingsRow label="保留期限" hint="0 为不清理；到期文件移入回收站">
      <div class="number-control">
        <input
          class="rule-input number"
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
    <SettingsRow label="导出后清理暂存副本" hint="导出成功后移出，可在时间线撤销">
      <ToggleSwitch
        label="导出后清理暂存副本"
        :model-value="security.cleanupAfterExport"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ cleanupAfterExport: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="禁止缩略图" hint="不生成缩略图预览">
      <ToggleSwitch
        label="禁止缩略图"
        :model-value="security.suppressThumbnails"
        :disabled="!security.enabled"
        @update:model-value="(value) => update({ suppressThumbnails: value })"
      />
    </SettingsRow>
    <div class="sep" />
    <SettingsRow label="禁止全文索引" hint="正文与 OCR 不进索引，文件名不受影响">
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
        <button type="button" class="rule-button" @click="refreshStatus">检查状态</button>
        <button v-if="status?.locked" type="button" class="rule-button primary" @click="unlockNow">
          解锁
        </button>
        <button v-else type="button" class="rule-button" @click="lockNow">立即锁定</button>
      </div>
    </div>
    <p class="disable-note">关闭敏感匣不解密已有文件；解密请用 Windows 文件属性。</p>
  </div>
</template>

<style scoped>
/* 无自身外壳：直接铺在所属匣卡片内，行距与分隔线对齐设置卡片规格。
   输入框与按钮统一走 settings.css 的 .rule-input / .rule-button。 */
.security-editor :deep(.row) {
  padding: 14px 0;
}
.security-status {
  display: grid;
  gap: 4px;
  margin: 14px 0 0;
  padding: 10px 12px;
  border-radius: 9px;
  background: var(--accent-soft);
  color: var(--ink-2);
  font-size: 11.5px;
}
.disable-note {
  margin: 10px 0 0;
  color: var(--ink-2);
  font-size: 11.5px;
}
.sep {
  height: 1px;
  margin: 0;
  background: var(--line);
}
.security-actions {
  display: flex;
  align-items: center;
  gap: 7px;
  margin-top: 5px;
}
</style>
