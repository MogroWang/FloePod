<script setup lang="ts">
/** 品牌图形：FloePod 纯 SVG Logo，按高度等比缩放。 */
import logoUrl from "@/assets/floepod.svg";
withDefaults(defineProps<{ size?: number }>(), { size: 24 });
</script>

<template>
  <img
    :src="logoUrl"
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