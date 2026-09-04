<script setup lang="ts">
/**
 * 右键菜单窗口视图（全局唯一 context_menu 窗口）。
 * 收到 SHOW 事件后渲染菜单、测量内容尺寸交由后端定位显示；
 * 用户选择 → 回传来源面板执行；失焦 / Escape → 关闭。
 * 窗口与菜单卡片同形（不留阴影余量，四角由后端按卡片圆角裁剪），
 * 点击卡片外任意位置都会让菜单失焦而关闭。
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import ContextMenu from "@/components/ContextMenu.vue";
import type { MenuItemSpec } from "@/domain/menu";
import type { Material } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";
import { useSettingsStore } from "@/stores/settings";

const settingsStore = useSettingsStore();
const seq = ref(0);
const podId = ref(0);
const items = ref<MenuItemSpec[]>([]);
const material = ref<Material>("plain");
const visible = ref(false);
const anchorEl = ref<HTMLElement | null>(null);
let disposeShow: (() => void) | null = null;
let closing = false;

async function measureAndShow() {
  await nextTick();
  // 必须用布局尺寸（offsetWidth/Height）：getBoundingClientRect 会带上
  // 弹入动画进行中的 transform 缩放，量出来的窗口比最终内容小，
  // 菜单卡片会被窗口边缘裁切。
  const anchor = anchorEl.value;
  const width = anchor?.offsetWidth ?? 232;
  const height = anchor?.offsetHeight ?? 200;
  await ipc.resizeContextMenu(seq.value, width, height).catch((err) => {
    console.error("context menu resize failed", err);
  });
}

function hide() {
  if (!visible.value) return;
  visible.value = false;
  void ipc
    .hideContextMenu(seq.value, podId.value)
    .catch((err) => console.error("context menu hide failed", err));
}

function onExecute(item: MenuItemSpec) {
  // 先取快照再置不可见：动作回传与关闭都要用同一代 seq / podId。
  const currentSeq = seq.value;
  const currentPod = podId.value;
  visible.value = false;
  void ipc
    .contextMenuChoice(currentSeq, currentPod, item)
    .catch((err) => console.error("context menu choice failed", err));
  closing = true;
  void ipc
    .hideContextMenu(currentSeq, currentPod)
    .catch((err) => console.error("context menu hide failed", err));
}

function onWindowBlur() {
  if (closing) {
    // 主动关闭触发的 blur 已由 hide 收尾，避免重复隐藏竞争。
    closing = false;
    return;
  }
  hide();
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === "Escape") hide();
}

onMounted(async () => {
  // 菜单窗口跟随应用主题（深浅色与面板一致）。
  await settingsStore.load().catch((err) => console.error("menu theme load failed", err));
  await ipc.contextMenuReady().catch((err) => console.error("menu ready failed", err));
  disposeShow = await listenCurrent<{
    seq: number;
    podId: number;
    items: MenuItemSpec[];
    material?: Material;
  }>(Events.ContextMenuShow, (payload) => {
    closing = false;
    seq.value = payload.seq;
    podId.value = payload.podId;
    items.value = payload.items;
    material.value = payload.material ?? "plain";
    visible.value = true;
    void measureAndShow();
  });
  window.addEventListener("blur", onWindowBlur);
  window.addEventListener("keydown", onKeydown);
});

onBeforeUnmount(() => {
  disposeShow?.();
  window.removeEventListener("blur", onWindowBlur);
  window.removeEventListener("keydown", onKeydown);
});
</script>

<template>
  <div class="menu-window" :data-material="material">
    <div v-if="visible" ref="anchorEl" class="menu-anchor">
      <ContextMenu :items="items" @execute="onExecute" />
    </div>
  </div>
</template>

<style scoped>
.menu-window {
  /* 全窗口透明；内容锚点与窗口左上角对齐（窗口与卡片同形，无余量） */
  width: 100vw;
  height: 100vh;
  overflow: hidden;
}
.menu-window[data-material="acrylic"],
.menu-window[data-material="mica"] {
  /* 原生系统材质已经负责模糊 / 云母纹理；WebView 只叠加适度主题着色。
     旧版 96% 近实心底色会把云母完全遮住，看起来像材质失效。 */
  --context-menu-surface: color-mix(in srgb, var(--surface) 76%, transparent);
}
.menu-window[data-material="plain"] {
  --context-menu-surface: color-mix(in srgb, var(--surface) 96%, transparent);
}
.menu-anchor {
  position: fixed;
  left: 0;
  top: 0;
}
</style>
