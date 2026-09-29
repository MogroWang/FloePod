<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { useSettingsStore } from "@/stores/settings";
import type { AccessibilitySettings } from "@/domain/types";
import SettingsRow from "@/components/SettingsRow.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import RangeSlider from "@/components/RangeSlider.vue";
import SafetyCenter from "@/components/SafetyCenter.vue";
const { s, save } = useSettingsEditor();
const settingsStore = useSettingsStore();
function saveAccessibility(patch: Partial<AccessibilitySettings>) {
  void save(() => ({ accessibility: { ...s.value.accessibility, ...patch } }));
}
/** 拖动中即时预览缩放；落库交给 commit。 */
function previewScale(value: number) {
  settingsStore.previewScale(value);
}
function commitScale(value: number) {
  saveAccessibility({ scale: value });
}
</script>
<template>
  <div>
    <h2 class="page-title">辅助功能</h2>
    <p class="page-desc">放大界面、减少干扰，并查看或恢复每一步文件操作。</p>
    <div class="settings-card safety-settings">
      <SettingsRow label="界面大小" hint="同时放大文字、按钮和点击目标">
        <div class="scale-control">
          <RangeSlider
            :value="s.accessibility.scale"
            :min="0.5"
            :max="1.5"
            :step="0.05"
            aria-label="界面大小"
            @update:value="previewScale"
            @commit="commitScale"
          />
          <span class="scale-value" aria-hidden="true">
            {{ Math.round(s.accessibility.scale * 100) }}%
          </span>
        </div>
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="高对比度" hint="使用黑底、白字和高可见焦点框">
        <ToggleSwitch
          label="高对比度"
          :model-value="s.accessibility.highContrast"
          @update:model-value="(value) => saveAccessibility({ highContrast: value })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="减少透明效果" hint="关闭模糊和玻璃效果，提高文字清晰度">
        <ToggleSwitch
          label="减少透明效果"
          :model-value="s.accessibility.reduceTransparency"
          @update:model-value="(value) => saveAccessibility({ reduceTransparency: value })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="减少动画" hint="关闭弹入、缩放和过渡动画">
        <ToggleSwitch
          label="减少动画"
          :model-value="s.accessibility.reduceMotion"
          @update:model-value="(value) => saveAccessibility({ reduceMotion: value })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow
        label="资源管理器“发送到 FloePod”"
        hint="为每个匣显示独立的发送选项，右键文件即可暂存"
      >
        <ToggleSwitch
          label="资源管理器发送到 FloePod"
          :model-value="s.accessibility.sendToMenu"
          @update:model-value="(value) => saveAccessibility({ sendToMenu: value })"
        />
      </SettingsRow>
    </div>
    <h3 class="section-title">操作时间线与一键恢复</h3>
    <SafetyCenter />
  </div>
</template>
<style scoped>
.scale-control {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 220px;
}
.scale-control :deep(input[type="range"]) {
  flex: 1;
}
.scale-value {
  min-width: 42px;
  text-align: right;
  font-size: 12px;
  font-weight: 550;
  font-variant-numeric: tabular-nums;
  color: var(--ink-2);
}
</style>
