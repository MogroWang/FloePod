<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { ref } from "vue";
import { ipc } from "@/ipc/client";
import SettingsRow from "@/components/SettingsRow.vue";
import HotkeyRecorder from "@/components/HotkeyRecorder.vue";
const { s, settingsStore, showToast } = useSettingsEditor();
const hotkeyError = ref("");
let hotkeySaveRevision = 0;
async function saveHotkey(
  key: "toggleBar" | "collectClipboard" | "openPanel" | "lockSensitive",
  combo: string,
) {
  const revision = ++hotkeySaveRevision;
  hotkeyError.value = "";
  try {
    await settingsStore.save(() => ({
      hotkeys: { ...s.value!.hotkeys, [key]: combo },
    }));
    if (revision === hotkeySaveRevision) hotkeyError.value = "";
  } catch (err) {
    if (revision !== hotkeySaveRevision) return;
    hotkeyError.value = `快捷键「${combo}」注册失败，可能与其他软件冲突`;
    showToast(hotkeyError.value);
  }
}

async function resetHotkeys() {
  const revision = ++hotkeySaveRevision;
  try {
    const defaults = await ipc.getHotkeyDefaults();
    // 获取默认值本身也是异步的；若此时用户已经录入了更新的快捷键，旧的
    // “重置”意图不能晚到并排在新值之后覆盖它。
    if (revision !== hotkeySaveRevision) return;
    await settingsStore.save({ hotkeys: defaults });
    if (revision === hotkeySaveRevision) {
      hotkeyError.value = "";
      showToast("已恢复默认快捷键");
    }
  } catch (err) {
    if (revision !== hotkeySaveRevision) return;
    console.error("reset hotkeys failed", err);
    showToast("重置失败，请重试");
  }
}
</script>
<template>
  <div>
    <h2 class="page-title">快捷键</h2>
    <p class="page-desc">全局快捷键，点击后按下新组合即可修改。</p>
    <div class="settings-card">
      <SettingsRow label="显示 / 隐藏全部匣">
        <HotkeyRecorder
          :model-value="s.hotkeys.toggleBar"
          @update:model-value="(v) => saveHotkey('toggleBar', v)"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="收集剪贴板文字" hint="把当前剪贴板里的文字存为第一匣的暂存">
        <HotkeyRecorder
          :model-value="s.hotkeys.collectClipboard"
          @update:model-value="(v) => saveHotkey('collectClipboard', v)"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="打开第一匣浮动面板">
        <HotkeyRecorder
          :model-value="s.hotkeys.openPanel"
          @update:model-value="(v) => saveHotkey('openPanel', v)"
        />
      </SettingsRow>
      <div class="sep" />
      <SettingsRow label="紧急锁定敏感匣" hint="立即清除所有内存中的敏感匣解锁状态">
        <HotkeyRecorder
          :model-value="s.hotkeys.lockSensitive"
          @update:model-value="(v) => saveHotkey('lockSensitive', v)"
        />
      </SettingsRow>
    </div>
    <p v-if="hotkeyError" class="error">{{ hotkeyError }}</p>
    <div class="reset-line">
      <button type="button" class="btn ghost" @click="resetHotkeys">恢复默认快捷键</button>
    </div>
  </div>
</template>
