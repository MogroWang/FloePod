<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { ref } from "vue";
import type { ThemeMode } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { THEMES } from "./options";
import SettingsRow from "@/components/SettingsRow.vue";
import SegmentedControl from "@/components/SegmentedControl.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
const { s, save, showToast } = useSettingsEditor();
const autostartBusy = ref(false);
async function saveAutostart(enabled: boolean) {
  if (autostartBusy.value) return;
  autostartBusy.value = true;
  try {
    await save({ autostart: enabled });
  } finally {
    autostartBusy.value = false;
  }
}

async function quitApp() {
  try {
    await ipc.quitApp();
  } catch (err) {
    console.error("quit app failed", err);
    showToast("退出失败，请从托盘重试");
  }
}
</script>
<template>
  <div>
    <h2 class="page-title">常规</h2>
    <p class="page-desc">浮匣的整体外观与行为。</p>
    <div class="settings-card">
      <SettingsRow label="主题" hint="跟随系统会随 Windows 深浅色自动切换">
        <SegmentedControl
          :options="THEMES"
          :model-value="s.theme"
          @update:model-value="(v) => save({ theme: v as ThemeMode })"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="开机自启" hint="以托盘常驻方式随 Windows 启动">
        <ToggleSwitch
          label="开机自启"
          :model-value="s.autostart"
          :disabled="autostartBusy"
          @update:model-value="saveAutostart"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="退出浮匣" hint="关闭所有匣并退出程序（托盘仍可退出）">
        <button type="button" class="btn danger" @click="quitApp">退出</button>
      </SettingsRow>
    </div>
  </div>
</template>
