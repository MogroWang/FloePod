<script setup lang="ts">
/**
 * 右键菜单窗口视图（全局唯一 context_menu 窗口）。
 * 收到 SHOW 事件后渲染菜单、测量内容尺寸交由后端定位显示；
 * 用户选择 → 回传来源面板执行；失焦 / Escape → 关闭。
 * 菜单四周留 12px 透明余量承载阴影，测量值需一并上报。
 */
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import ContextMenu from "@/components/ContextMenu.vue";
import type { MenuItemSpec } from "@/domain/menu";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";
import { useSettingsStore } from "@/stores/settings";

const MENU_MARGIN = 12;

const settingsStore = useSettingsStore();
const seq = ref(0);
const podId = ref(0);
const items = ref<MenuItemSpec[]>([]);
const visible = ref(false);
const anchorEl = ref<HTMLElement | null>(null);
let disposeShow: (() => void) | null = null;
let closing = false;

async function measureAndShow() {
  await nextTick();
  const rect = anchorEl.value?.getBoundingClientRect();
  const width = (rect?.width ?? 232) + MENU_MARGIN * 2;
  const height = (rect?.height ?? 200) + MENU_MARGIN * 2;
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
  }>(Events.ContextMenuShow, (payload) => {
    closing = false;
    seq.value = payload.seq;
    podId.value = payload.podId;
    items.value = payload.items;
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
  <div class="menu-window">
    <div v-if="visible" ref="anchorEl" class="menu-anchor">
      <ContextMenu :items="items" @execute="onExecute" />
    </div>
  </div>
</template>

<style scoped>
.menu-window {
  /* 全窗口透明；内容锚点带 12px 阴影余量 */
  width: 100vw;
  height: 100vh;
  overflow: hidden;
}
.menu-anchor {
  position: fixed;
  left: 12px;
  top: 12px;
}
</style>
