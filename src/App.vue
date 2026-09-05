<script setup lang="ts">
/**
 * 多窗口共享一个 Vue 入口：按 Tauri 窗口 label 选择视图。
 * - settings          -> 设置 / OOBE
 * - pod_{id}          -> 匣的边缘浮动条（贴在屏幕边缘）
 * - pod_{id}_panel    -> 匣的弹出浮动面板
 * - context_menu      -> 全局右键菜单
 * 浏览器开发时用 location.hash（#/settings /#/pod_1 /#/pod_1_panel）。
 */
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { parseWindowLabel } from "@/domain/windowLabel";
import ContextMenuWindow from "@/windows/ContextMenuWindow.vue";
import PodBar from "@/windows/PodBar.vue";
import PodPanel from "@/windows/PodPanel.vue";
import SettingsWindow from "@/windows/SettingsWindow.vue";

/**
 * 窗口标签必须在首帧渲染前确定。旧实现先渲染 pod_1，再在 onMounted
 * 中切换真实窗口，导致所有动态窗口短暂挂载错误组件并遗留事件监听。
 */
function resolveWindowLabel(): string {
  if ("__TAURI_INTERNALS__" in window) {
    return getCurrentWebviewWindow().label;
  }
  return location.hash.replace(/^#\/?/, "") || "pod_1";
}

const label = resolveWindowLabel();
const target = parseWindowLabel(label);
const view = target?.kind === "settings"
  ? SettingsWindow
  : target?.kind === "podPanel"
    ? PodPanel
    : target?.kind === "podBar"
      ? PodBar
      : target?.kind === "contextMenu"
        ? ContextMenuWindow
        : null;
const viewProps = target && "podId" in target ? { podId: target.podId } : {};
</script>

<template>
  <component v-if="view" :is="view" v-bind="viewProps" />
  <main v-else class="window-error" role="alert">
    无法识别窗口：{{ label }}
  </main>
</template>

<style scoped>
.window-error {
  box-sizing: border-box;
  min-height: 100vh;
  padding: 16px;
  color: #ffb4ab;
  background: #211a1a;
  font: 14px/1.5 system-ui, sans-serif;
}
</style>
