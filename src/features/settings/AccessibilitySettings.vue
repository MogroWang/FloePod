<script setup lang="ts">
import { useSettingsEditor } from "./context";
import type { AccessibilitySettings } from "@/domain/types";
import SettingsRow from "@/components/SettingsRow.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import SafetyCenter from "@/components/SafetyCenter.vue";
import SearchCenter from "@/components/SearchCenter.vue";
import OrganizationCenter from "@/components/OrganizationCenter.vue";
const { s, save } = useSettingsEditor();
function saveAccessibility(patch: Partial<AccessibilitySettings>) {
  void save(() => ({ accessibility: { ...s.value.accessibility, ...patch } }));
}
</script>
<template>
  <div>
    <h2 class="page-title">辅助功能</h2>
    <p class="page-desc">放大界面、减少干扰，并查看或恢复每一步文件操作。</p>
    <div class="settings-card safety-settings">
      <SettingsRow label="界面大小" hint="同时放大文字、按钮和点击目标">
        <select
          class="input compact-select"
          :value="s.accessibility.scale"
          aria-label="辅助功能界面大小"
          @change="saveAccessibility({ scale: Number(($event.target as HTMLSelectElement).value) })"
        >
          <option :value="1">100%</option>
          <option :value="1.25">125%</option>
          <option :value="1.5">150%</option>
          <option :value="2">200%</option>
        </select>
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
      <SettingsRow label="简明语言" hint="用完整问题代替术语和仅图标提示">
        <ToggleSwitch
          label="简明语言"
          :model-value="s.accessibility.simpleLanguage"
          @update:model-value="(value) => saveAccessibility({ simpleLanguage: value })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="危险操作确认" hint="移动、批量移出前始终显示将要发生的事情">
        <ToggleSwitch
          label="危险操作确认"
          :model-value="s.accessibility.confirmDangerous"
          @update:model-value="(value) => saveAccessibility({ confirmDangerous: value })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow
        label="资源管理器“发送到 FloePod”"
        hint="右键文件即可复制到第一个可用匣，作为拖拽替代"
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
    <h3 class="section-title search-section-title">本地 OCR、全文搜索与标签</h3>
    <SearchCenter />
    <h3 class="section-title search-section-title">机构策略、审计与诊断</h3>
    <OrganizationCenter />
  </div>
</template>
