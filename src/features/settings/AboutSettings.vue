<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { openUrl } from "@tauri-apps/plugin-opener";
import BrandMark from "@/components/BrandMark.vue";
const { s, showToast } = useSettingsEditor();
async function checkUpdates() {
  try {
    await openUrl("https://github.com/MogroWang/FloePod/releases/latest");
  } catch (error) {
    showToast(`无法打开版本页面：${String(error)}`);
  }
}
</script>
<template>
  <div>
    <div class="about-hero">
      <BrandMark :size="48" class="about-brand" />
      <div class="about-ver">版本 {{ s.version }}</div>
    </div>
    <p class="about-text">
      本地优先的屏幕边缘暂存工具：拖进来集中保管，拖出去继续使用。<br />
      不联网、不收集数据，所有内容只存在你自己的电脑上。
    </p>
    <div class="about-meta">
      <div class="about-row">
        <span class="about-key">数据位置</span>
        <span class="about-val">{{ s.dataDir }}</span>
      </div>
      <div class="about-row">
        <span class="about-key">匣的数量</span>
        <span class="about-val">{{ s.pods.length }} 个</span>
      </div>
    </div>
    <div class="reset-line about-update-line">
      <button type="button" class="btn" @click="checkUpdates">查看最新版本与下载</button>
    </div>
  </div>
</template>
