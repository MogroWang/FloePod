<script setup lang="ts">
/**
 * 品牌图形，按高度等比缩放。
 * - wordmark（默认）：横版 SVG 字标，适合关于页 / 首次运行向导
 * - icon：方形应用图标，适合标题栏 / 浮动条等紧凑位置
 */
import iconUrl from "../../app-icon.png";
import wordmarkUrl from "@/assets/floepod.svg";
withDefaults(defineProps<{ size?: number; mark?: "wordmark" | "icon" }>(), {
  size: 24,
  mark: "wordmark",
});
</script>

<template>
  <img
    :src="mark === 'icon' ? iconUrl : wordmarkUrl"
    :style="{ height: size + 'px' }"
    class="brand-logo"
    alt="FloePod"
    draggable="false"
  />
</template>

<style scoped>
.brand-logo {
  width: auto;
  display: block;
  user-select: none;
}
</style>
