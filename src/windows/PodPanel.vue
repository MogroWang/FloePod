<script setup lang="ts">
/**
 * 单个「匣」的弹出浮动面板：列表 / 拖入询问 / 冲突解决 三种模式。
 * 不抢焦点显示（Rust 侧 SW_SHOWNOACTIVATE），浮动面板材质恒定全量下发、
 * 不随焦点降级；指针离开超时后淡出隐藏（Rust 看门狗），
 * 重新悬停或主动弹出时淡入。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { useUnlisteners } from "@/composables/useUnlisteners";
import { useToast } from "@/composables/useToast";
import type { PanelMode, PanelState } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";
import { clampOpacity } from "@/lib/format";
import { useSettingsStore } from "@/stores/settings";
import { useStagingStore } from "@/stores/staging";
import ItemRow from "@/components/ItemRow.vue";
import ActionChooser from "@/components/ActionChooser.vue";
import ConflictDialog from "@/components/ConflictDialog.vue";
import ContextMenu from "@/components/ContextMenu.vue";
import SegmentedControl from "@/components/SegmentedControl.vue";
import TrustExportDialog from "@/components/TrustExportDialog.vue";

import { usePanelSelection } from "@/features/panel/usePanelSelection";
import { usePanelDrag } from "@/features/panel/usePanelDrag";
import { usePanelSecurity } from "@/features/panel/usePanelSecurity";
import { usePanelText } from "@/features/panel/usePanelText";
import { usePanelExport } from "@/features/panel/usePanelExport";
import { usePanelFiles } from "@/features/panel/usePanelFiles";
import { usePanelInput } from "@/features/panel/usePanelInput";
import { usePanelMenu } from "@/features/panel/usePanelMenu";
import { usePanelLayout } from "@/features/panel/usePanelLayout";
import type { PanelContext } from "@/features/panel/context";

const props = defineProps<{ podId: number }>();

const settingsStore = useSettingsStore();
const staging = useStagingStore();

const pod = computed(() => settingsStore.pod(props.podId));
const panelStyle = computed<Record<string, string>>(() => {
  // 浮动面板填充色与不透明度独立于边缘浮动条；旧配置缺失时回退匣的不透明度。
  const opacity = clampOpacity(pod.value?.panelOpacity ?? pod.value?.opacity);
  const fill = pod.value?.panelColor?.trim() || "var(--surface)";
  return { "--pod-opacity": `${opacity * 100}%`, "--pod-fill": fill };
});

const mode = ref<PanelMode>("list");
const pendingPaths = ref<string[]>([]);
const trustOpen = ref(false);
let securityTimer: number | undefined;
const { retainUnlistener, disposeUnlisteners, isMounted } = useUnlisteners();

const pinned = ref(false);
const pinBusy = ref(false);
const listActionBusy = ref(false);
/** 导出 / 删除 / 拖出都会按启动时的选择或文件状态处理；三者任一进行中即视为忙碌。 */
const anyBusy = computed(
  () =>
    exportBusy.value ||
    listActionBusy.value ||
    dragBusy.value ||
    textBusy.value ||
    askBusy.value ||
    unlocking.value,
);
let lastFadeIn = Number.NEGATIVE_INFINITY;
let modeRevision = 0;
let pinRevision = 0;
const { toast, showToast, disposeToast } = useToast(2200, isMounted);

const context: PanelContext = {
  podId: () => props.podId,
  staging,
  settingsStore,
  showToast,
  refreshAfterMutation,
  isMounted,
  isBusy: () => anyBusy.value,
};
const {
  items,
  selectedCount,
  currentSelectedIds,
  clearSelection,
  selectAll,
  onSelect,
  selectedOrSingle,
  resetAnchor,
} = usePanelSelection(context);
const { textOpen, textTitle, textValue, textBusy, stashText, pasteClipboard } =
  usePanelText(context);
const {
  securityStatus,
  unlocking,
  sensitiveLocked,
  refreshSecurityStatus,
  unlockSensitivePod,
  lockSensitivePod,
  applyLockChanged,
} = usePanelSecurity(context, pod);
const { dragMode, dragBusy, onDragOut } = usePanelDrag(context, clearSelection);
const { conflict, exportBusy, exportSelected, resolveConflict, cancelConflict } = usePanelExport(
  context,
  mode,
  currentSelectedIds,
  resetAnchor,
);
const { openItem, revealItem, removeItem, removeSelected, removeIds, confirmClear, clearAll } =
  usePanelFiles(context, listActionBusy, selectedCount, currentSelectedIds, resetAnchor);
const { askBusy, pickPaths, pasteClipboardFiles, chooseAction, cancelAsk } = usePanelInput(
  context,
  pod,
  listActionBusy,
  mode,
  pendingPaths,
);
const {
  menuOpen,
  closeInlineMenu,
  inlineMenu,
  inlineMenuStyle,
  onItemContextMenu,
  runMenuAction,
  executeInlineMenu,
  onGlobalPointerDown,
} = usePanelMenu(context, onSelect, removeIds);
const { rootEl, bindHead, bindList, bindContent, bindFoot, scheduleResize, observeContent } =
  usePanelLayout(context, mode, textOpen);

function playFadeIn() {
  const el = rootEl.value;
  if (!el) return;
  // 首挂载时 onMounted 与 PANEL_SHOWN 会先后触发，短窗内去重避免动画重播闪烁
  const now = performance.now();
  if (now - lastFadeIn < 100) return;
  lastFadeIn = now;
  // 清除隐藏阶段遗留的淡出态，再从头播放淡入
  el.classList.remove("panel-fade-out", "panel-fade-in");
  void el.offsetWidth;
  el.classList.add("panel-fade-in");
}

async function onTogglePinned() {
  if (pinBusy.value) return;
  const previous = pinned.value;
  const next = !previous;
  // 防止较早的 getPanelState 响应撤销刚执行的本地命令。
  const request = ++pinRevision;
  pinBusy.value = true;
  pinned.value = next;
  try {
    await ipc.setPanelPinned(props.podId, next);
  } catch (err) {
    if (request === pinRevision) pinned.value = previous;
    console.error("pin update failed", err);
    showToast("固定状态更新失败，请重试");
  } finally {
    pinBusy.value = false;
  }
}

function applyPanelMode(nextMode: PanelMode, paths: string[] = []) {
  // 冲突目标和选择信息只存在于当前 WebView；若中途重建，只能回到列表。
  if (nextMode === "conflict" && !conflict.value) {
    mode.value = "list";
    pendingPaths.value = [];
    void ipc.setPanelMode(props.podId, "list").catch((err) => {
      console.error("recover stale conflict mode failed", err);
    });
    return;
  }

  // 询问页缺少待处理路径时无法继续，回到列表并同步修复后端状态。
  if (nextMode === "ask" && paths.length === 0) {
    mode.value = "list";
    pendingPaths.value = [];
    void ipc.setPanelMode(props.podId, "list").catch((err) => {
      console.error("recover empty pending drop failed", err);
    });
    return;
  }

  mode.value = nextMode;
  pendingPaths.value = nextMode === "ask" ? [...paths] : [];
  if (nextMode !== "conflict") conflict.value = null;
  if (nextMode !== "list") textOpen.value = false;
}

function applyPanelState(state: PanelState) {
  applyPanelMode(state.mode, state.paths);
  pinned.value = state.pinned;
}

async function syncPanelState() {
  const expectedModeRevision = modeRevision;
  const expectedPinRevision = pinRevision;
  try {
    const state = await ipc.getPanelState(props.podId);
    if (!isMounted()) return;
    // 读取期间收到的事件更新更晚，不能再用旧响应覆盖。
    if (modeRevision === expectedModeRevision) applyPanelMode(state.mode, state.paths);
    if (pinRevision === expectedPinRevision) pinned.value = state.pinned;
  } catch (err) {
    console.error("panel state snapshot failed", err);
  }
}

async function refreshAfterMutation(label: string): Promise<boolean> {
  try {
    await staging.refresh(props.podId);
    return true;
  } catch (err) {
    console.error(`${label} succeeded but list refresh failed`, err);
    showToast(`${label}已完成，但列表刷新失败`);
    return false;
  }
}

async function openTrustCenter() {
  if (!selectedCount.value || anyBusy.value) return;
  trustOpen.value = true;
  await ipc.setDraggingOut(props.podId, true).catch(() => {});
  scheduleResize();
}

async function closeTrustCenter() {
  trustOpen.value = false;
  await ipc.setDraggingOut(props.podId, false).catch(() => {});
  scheduleResize();
}

function completeTrustAction(message: string) {
  showToast(message);
  void closeTrustCenter();
}

function onKeydown(e: KeyboardEvent) {
  const target = e.target as HTMLElement;
  if (target.tagName === "TEXTAREA" || target.tagName === "INPUT") {
    if (e.key === "Escape") (e.target as HTMLElement).blur();
    return;
  }
  if (e.key === "Escape") {
    if (trustOpen.value) void closeTrustCenter();
    else if (mode.value === "conflict") void cancelConflict();
    else if (mode.value === "ask") void cancelAsk();
    else if (textOpen.value) textOpen.value = false;
    else if (selectedCount.value) clearSelection();
    else {
      void ipc.hidePanel(props.podId).catch((err) => console.error("hide panel failed", err));
    }
  } else if (e.ctrlKey && e.key.toLowerCase() === "a") {
    e.preventDefault();
    if (mode.value === "list" && !textOpen.value && !anyBusy.value) selectAll();
  } else if (e.ctrlKey && e.key.toLowerCase() === "v" && mode.value === "list" && !textOpen.value) {
    e.preventDefault();
    void pasteClipboardFiles();
  } else if (
    e.key === "Delete" &&
    mode.value === "list" &&
    !textOpen.value &&
    selectedCount.value
  ) {
    void removeSelected();
  }
}

watch(
  () => [mode.value, items.value.length, selectedCount.value, pod.value?.panelWidth],
  () => scheduleResize(),
);

onMounted(async () => {
  mode.value = "list";
  pendingPaths.value = [];
  pinned.value = false;
  staging.setActivePod(props.podId);

  // 先注册定向事件，再主动读取运行态快照。Promise.allSettled 可确保单个
  // 监听失败时，已成功注册的监听仍然会被保留并在卸载时释放。
  const registrations = await Promise.allSettled([
    staging.listenChanges(props.podId),
    listenCurrent(Events.PanelMode, (p) => {
      modeRevision += 1;
      applyPanelMode(p.mode, p.paths ?? []);
    }),
    listenCurrent(Events.PanelState, (state) => {
      modeRevision += 1;
      pinRevision += 1;
      applyPanelState(state);
    }),
    /* 浮动面板每次出现都重播淡入动画 */
    listenCurrent(Events.PanelShown, () => playFadeIn()),
    /* 固定状态同步 */
    listenCurrent(Events.PanelPinned, (p) => {
      pinRevision += 1;
      pinned.value = p.pinned;
    }),
    /* 右键菜单窗口回传的用户选择 */
    listenCurrent(Events.ContextMenuChoice, (p) => {
      if (p.podId !== props.podId) return;
      void runMenuAction(p.action);
    }),
    /* 右键菜单已关闭：解除拖出保活 */
    listenCurrent(Events.ContextMenuClosed, (p) => {
      if (p.podId !== props.podId || !menuOpen.value) return;
      menuOpen.value = false;
      void ipc
        .setDraggingOut(props.podId, false)
        .catch((err) => console.error("menu presence restore failed", err));
    }),
    /* 辅助功能 Alt+数字：打开对应浮动面板后直接提供非拖拽文件选择器。 */
    listenCurrent(Events.RequestFilePicker, (p) => {
      if (p.podId !== props.podId) return;
      void pickPaths(false);
    }),
    listenCurrent(Events.PodLockChanged, (p) => {
      if (p.podId !== props.podId) return;
      applyLockChanged(p.locked);
    }),
    /* 浮动面板开始隐藏：先播放淡出，后端延迟 220ms 再隐藏原生窗口。
       运行态由其他定向事件同步，不能在此清空询问或冲突。 */
    listenCurrent(Events.PanelHidden, () => {
      const el = rootEl.value;
      if (!el) return;
      el.classList.remove("panel-fade-in");
      el.classList.add("panel-fade-out");
    }),
  ]);
  for (const result of registrations) {
    if (result.status === "fulfilled") retainUnlistener(result.value);
    else console.error("panel listener registration failed", result.reason);
  }
  if (!isMounted()) return;

  await syncPanelState();
  try {
    await settingsStore
      .listenChanges()
      .catch((err) => console.error("settings listener failed", err));
    await settingsStore.load();
    await refreshSecurityStatus();
    if (!sensitiveLocked.value) await staging.refresh(props.podId);
  } catch (err) {
    console.error("pod panel initialization failed", err);
    showToast("浮动面板内容加载失败，请重新打开");
  }
  if (!isMounted()) return;

  window.addEventListener("keydown", onKeydown);
  window.addEventListener("pointerdown", onGlobalPointerDown, true);
  securityTimer = window.setInterval(refreshSecurityStatus, 30_000);

  observeContent();

  await nextTick();
  scheduleResize();
  playFadeIn();
});

onBeforeUnmount(() => {
  disposeUnlisteners();
  disposeToast();
  window.removeEventListener("keydown", onKeydown);
  window.removeEventListener("pointerdown", onGlobalPointerDown, true);
  window.clearInterval(securityTimer);
});

function onPointerEnter() {
  void ipc
    .reportPresence(props.podId, "panel", true)
    .catch((err) => console.error("panel presence update failed", err));
}
function onPointerLeave() {
  void ipc
    .reportPresence(props.podId, "panel", false)
    .catch((err) => console.error("panel presence update failed", err));
}

async function openSettings() {
  try {
    await ipc.openSettings();
  } catch (err) {
    console.error("open settings failed", err);
    showToast("无法打开设置");
  }
}

// ---- 右键菜单（文件操作）----
</script>

<template>
  <div
    ref="rootEl"
    class="panel-root"
    :style="panelStyle"
    @pointerenter="onPointerEnter"
    @pointerleave="onPointerLeave"
  >
    <header :ref="bindHead" class="panel-head">
      <div class="pod-title">
        <div class="pod-name" :title="pod?.name">{{ pod?.name ?? "匣" }}</div>
        <span v-if="items.length" class="item-count">{{ items.length }}</span>
      </div>
      <div class="head-right">
        <span
          v-if="securityStatus?.expiresSoon"
          class="expiry-badge"
          :title="`${securityStatus.expiresSoon} 项已达到规则提醒或保留期限`"
        >
          到期 {{ securityStatus.expiresSoon }}
        </span>
        <button
          v-if="pod?.security.enabled && !sensitiveLocked"
          type="button"
          class="head-btn"
          title="立即锁定敏感匣"
          aria-label="立即锁定敏感匣"
          @click="lockSensitivePod"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <rect x="5" y="10" width="14" height="10" rx="2" />
            <path d="M8 10V7a4 4 0 0 1 8 0v3" />
          </svg>
        </button>
        <button
          v-if="mode === 'list' && !textOpen"
          type="button"
          class="head-btn"
          title="选择文件暂存"
          aria-label="选择文件暂存，不需要拖拽"
          @click="pickPaths(false)"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 5v14M5 12h14" />
          </svg>
        </button>
        <button
          v-if="mode === 'list' && !textOpen"
          type="button"
          class="head-btn"
          title="选择文件夹暂存"
          aria-label="选择文件夹暂存，不需要拖拽"
          @click="pickPaths(true)"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M3 7h6l2 2h10v10H3z" />
          </svg>
        </button>
        <button
          v-if="mode === 'list' && !textOpen"
          type="button"
          class="head-btn"
          title="暂存一段文字"
          aria-label="暂存一段文字"
          @click="textOpen = true"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
          >
            <path d="M4 7h16M4 12h10M4 17h7" />
          </svg>
        </button>
        <button
          type="button"
          class="head-btn"
          :class="{ on: pinned }"
          :disabled="pinBusy"
          :aria-pressed="pinned"
          :title="
            pinned ? '已固定，移开鼠标浮动面板保持展开' : '固定浮动面板（移开鼠标后保持展开）'
          "
          @click="onTogglePinned"
        >
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M12 17v5" />
            <path
              d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z"
            />
          </svg>
        </button>
        <button type="button" class="head-btn" title="设置" @click="openSettings">
          <svg
            width="14"
            height="14"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.7"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <circle cx="12" cy="12" r="3" />
            <path
              d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09a1.7 1.7 0 0 0 1.55-1 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.09a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55h.09a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.09a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.55 1Z"
            />
          </svg>
        </button>
      </div>
    </header>

    <div :ref="bindList" class="panel-body">
      <div :ref="bindContent" class="panel-content">
        <section v-if="sensitiveLocked" class="locked-panel" aria-labelledby="locked-title">
          <svg
            width="34"
            height="34"
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.5"
            aria-hidden="true"
          >
            <rect x="4" y="10" width="16" height="11" rx="2" />
            <path d="M8 10V7a4 4 0 0 1 8 0v3" />
          </svg>
          <h2 id="locked-title">敏感匣已锁定</h2>
          <p>
            文件由 Windows EFS 在磁盘上保护。通过 Windows Hello 或系统 PIN
            后才能查看、搜索、复制或导出。
          </p>
          <button
            type="button"
            class="act primary"
            :disabled="unlocking"
            @click="unlockSensitivePod"
          >
            {{ unlocking ? "正在验证…" : "使用 Windows Hello 解锁" }}
          </button>
        </section>

        <TrustExportDialog
          v-else-if="trustOpen"
          :ids="currentSelectedIds()"
          :pod-name="pod?.name ?? '匣'"
          @close="closeTrustCenter"
          @completed="completeTrustAction"
        />

        <ActionChooser
          v-else-if="mode === 'ask' && pendingPaths.length"
          :paths="pendingPaths"
          :busy="askBusy"
          @choose="chooseAction"
          @cancel="cancelAsk"
        />

        <ConflictDialog
          v-else-if="mode === 'conflict' && conflict"
          :names="conflict.names"
          :mode="conflict.mode"
          :busy="exportBusy"
          @resolve="resolveConflict"
          @cancel="cancelConflict"
        />

        <template v-else>
          <Transition name="stash-swap" mode="out-in">
            <div v-if="textOpen" key="text-stash" class="text-stash">
              <label class="text-field">
                <span>文件标题</span>
                <input
                  v-model="textTitle"
                  maxlength="48"
                  placeholder="可选，默认取正文第一行前 10 个字"
                  :disabled="textBusy"
                  autofocus
                  @keydown.enter.prevent
                />
              </label>
              <div class="text-field">
                <div class="text-field-head">
                  <label class="text-field-label" for="stash-text-body">正文</label>
                  <button
                    type="button"
                    class="text-clip-btn"
                    :disabled="textBusy"
                    title="读取剪贴板文字填入正文"
                    @click="pasteClipboard"
                  >
                    <svg
                      width="12"
                      height="12"
                      viewBox="0 0 24 24"
                      fill="none"
                      stroke="currentColor"
                      stroke-width="1.7"
                      stroke-linecap="round"
                      stroke-linejoin="round"
                      aria-hidden="true"
                    >
                      <rect x="8" y="3" width="8" height="4" rx="1" />
                      <path
                        d="M16 5h2a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h2"
                      />
                    </svg>
                    获取剪贴板
                  </button>
                </div>
                <textarea
                  id="stash-text-body"
                  v-model="textValue"
                  placeholder="粘贴或输入要暂存的文字…"
                  rows="5"
                  :disabled="textBusy"
                />
              </div>
              <div class="text-actions">
                <button type="button" class="act primary" :disabled="textBusy" @click="stashText">
                  {{ textBusy ? "暂存中…" : "暂存" }}
                </button>
                <button
                  type="button"
                  class="act ghost"
                  :disabled="textBusy"
                  @click="textOpen = false"
                >
                  取消
                </button>
              </div>
            </div>

            <div v-else key="list-view" class="list-view">
              <div v-if="items.length === 0" class="empty">
                <div class="empty-title">「{{ pod?.name ?? "匣" }}」是空的</div>
                <div class="empty-hint">
                  把文件或图片拖到屏幕边缘的这个匣上<br />将按当前匣的动作设置暂存
                </div>
              </div>
              <TransitionGroup
                v-else
                name="list"
                tag="div"
                class="items"
                role="listbox"
                aria-label="暂存项目"
                aria-multiselectable="true"
              >
                <ItemRow
                  v-for="item in items"
                  :key="item.id"
                  :item="item"
                  :selected="staging.selectedIds.has(item.id)"
                  :get-drag-paths="() => selectedOrSingle(item)"
                  @select="onSelect"
                  @open="openItem"
                  @reveal="revealItem"
                  @remove="removeItem"
                  @context-menu="onItemContextMenu"
                  @drag-out="onDragOut"
                />
              </TransitionGroup>
            </div>
          </Transition>
        </template>
      </div>
    </div>

    <footer v-if="mode === 'list' && !textOpen" :ref="bindFoot" class="panel-foot">
      <template v-if="selectedCount > 0">
        <span class="sel-count">已选 {{ selectedCount }} 项</span>
        <div class="foot-actions">
          <button
            type="button"
            class="foot-btn"
            :disabled="anyBusy"
            @click="exportSelected('copy')"
          >
            复制到…
          </button>
          <button
            type="button"
            class="foot-btn"
            :disabled="anyBusy"
            @click="exportSelected('move')"
          >
            移动到…
          </button>
          <button type="button" class="foot-btn" :disabled="anyBusy" @click="openTrustCenter">
            安全交接…
          </button>
          <button type="button" class="foot-btn danger" :disabled="anyBusy" @click="removeSelected">
            移出
          </button>
          <button type="button" class="foot-btn ghost" :disabled="anyBusy" @click="clearSelection">
            取消
          </button>
        </div>
      </template>
      <template v-else>
        <div class="foot-left">
          <span class="drag-mode-label">拖出时：</span>
          <SegmentedControl
            :options="[
              { value: 'copy', label: '复制' },
              { value: 'move', label: '剪切' },
            ]"
            v-model="dragMode"
            :disabled="anyBusy"
          />
        </div>
        <div v-if="items.length > 0" class="foot-right">
          <button type="button" class="foot-btn ghost" :disabled="anyBusy" @click="selectAll">
            全选
          </button>
          <button type="button" class="foot-btn ghost danger" :disabled="anyBusy" @click="clearAll">
            {{ confirmClear ? "确认清空？" : "清空" }}
          </button>
        </div>
      </template>
    </footer>

    <Transition name="toast">
      <div v-if="toast" class="toast" role="status" aria-live="polite">{{ toast }}</div>
    </Transition>

    <!-- 菜单窗口不可用时的内嵌降级菜单（浏览器预览 / 就绪前） -->
    <Teleport to="body">
      <div v-if="inlineMenu" class="inline-menu-layer" @pointerdown.self="closeInlineMenu">
        <div class="inline-menu-pos" :style="inlineMenuStyle">
          <ContextMenu :items="inlineMenu.items" @execute="executeInlineMenu" />
        </div>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
.panel-root {
  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: color-mix(
    in srgb,
    var(--pod-fill, var(--surface)) var(--pod-opacity, 100%),
    transparent
  );
  border-radius: var(--radius-panel);
  border: 1px solid var(--glass-line);
  box-shadow:
    var(--shadow-panel),
    inset 0 1px 0 var(--glass-inner);
  backdrop-filter: blur(24px) saturate(1.16);
  clip-path: inset(0 round var(--radius-panel));
  overflow: clip;
  box-sizing: border-box;
}
/* 显示动画：淡入 + 轻微缩放；悬停重新展开、拖入弹出与主动弹出统一 */
.panel-root.panel-fade-in {
  animation: panel-fade-in 260ms var(--ease-out) both;
}
@keyframes panel-fade-in {
  from {
    opacity: 0;
    transform: scale(0.985);
  }
}
/* 自动隐藏：先淡出（后端延迟 220ms 才隐藏原生窗口），之后 forwards 保持
   透明，下次显示第一帧不闪现完整内容 */
.panel-root.panel-fade-out {
  animation: panel-fade-out 220ms ease both;
}
@keyframes panel-fade-out {
  to {
    opacity: 0;
  }
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px 6px;
  flex-shrink: 0;
  /* 背景与浮动面板主体共用一整块半透明表面，不再单独分割出标题栏底色 */
}
.pod-title {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 7px;
}
.pod-name {
  font-size: 13px;
  font-weight: 600;
  color: var(--ink);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 180px;
}
.item-count {
  flex-shrink: 0;
  min-width: 18px;
  height: 18px;
  padding: 0 5px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  box-sizing: border-box;
  border-radius: 999px;
  background: var(--surface-2);
  color: var(--ink-2);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.head-right {
  display: flex;
  align-items: center;
  gap: 2px;
}
.expiry-badge {
  margin-right: 3px;
  padding: 2px 6px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--danger) 14%, transparent);
  color: var(--danger);
  font-size: 10px;
  white-space: nowrap;
}
.head-btn {
  border: 0;
  background: transparent;
  width: 28px;
  height: 28px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ink-2);
  cursor: pointer;
  transition:
    background 120ms ease,
    color 120ms ease;
}
.head-btn:hover {
  background: var(--surface-2);
  color: var(--ink);
}
.head-btn.on {
  color: var(--accent);
}
.head-btn:disabled {
  cursor: wait;
  opacity: 0.58;
}

.panel-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 4px 8px;
}
.panel-content {
  min-width: 0;
}
.locked-panel {
  display: grid;
  justify-items: center;
  gap: 10px;
  padding: 28px 18px;
  text-align: center;
}
.locked-panel svg {
  color: var(--accent);
}
.locked-panel h2,
.locked-panel p {
  margin: 0;
}
.locked-panel h2 {
  font-size: 16px;
}
.locked-panel p {
  max-width: 330px;
  color: var(--ink-2);
  font-size: 12px;
}

.empty {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 8px;
  padding: 44px 20px;
  text-align: center;
}
.empty-title {
  font-size: 14px;
  font-weight: 600;
  color: var(--ink);
}
.empty-hint {
  font-size: 12px;
  line-height: 1.7;
  color: var(--ink-3);
}

.items {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.list-enter-active {
  transition:
    opacity 220ms ease,
    transform 280ms var(--ease-out);
}
.list-leave-active {
  transition: opacity 140ms ease;
  position: absolute;
  width: calc(100% - 16px);
}
.list-enter-from {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
.list-leave-to {
  opacity: 0;
}
.list-move {
  transition: transform 280ms var(--ease-out);
}

.panel-foot {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  padding: 8px 12px 10px;
  min-height: 44px;
}
.sel-count {
  font-size: 12px;
  color: var(--ink-2);
  font-weight: 550;
  white-space: nowrap;
}
.foot-actions,
.foot-right {
  display: flex;
  gap: 6px;
  align-items: center;
}
.foot-left {
  display: flex;
  align-items: center;
  gap: 6px;
  min-width: 0;
}
.drag-mode-label {
  flex-shrink: 0;
  color: var(--ink-2);
  font-size: 12px;
  font-weight: 550;
  white-space: nowrap;
}
.foot-btn {
  border: 1px solid var(--line-strong);
  background: var(--surface-raised);
  color: var(--ink);
  border-radius: 8px;
  padding: 5px 10px;
  font-size: 12px;
  font-weight: 520;
  cursor: pointer;
  font-family: inherit;
  transition:
    transform 100ms ease,
    background 120ms ease;
}
.foot-btn:active {
  transform: scale(0.97);
}
.foot-btn:disabled,
.act:disabled {
  cursor: wait;
  opacity: 0.58;
  transform: none;
}
.foot-btn:hover {
  background: var(--surface-2);
}
.foot-btn.ghost {
  border-color: transparent;
  color: var(--ink-2);
}
.foot-btn.ghost:hover {
  border-color: var(--line-strong);
}
.foot-btn.danger {
  color: var(--danger);
}
.foot-btn.ghost.danger:hover {
  border-color: color-mix(in oklab, var(--danger) 45%, transparent);
}

.text-stash {
  padding: 14px 12px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.list-view {
  min-width: 0;
}
/* 列表 <-> 文字暂存的模式切换：先收起旧视图再展开新视图（out-in）。
   进入慢而远、离开快而短，全程用 --ease-out 出程缓动，观感流畅不生硬。 */
.stash-swap-enter-active {
  transition:
    opacity 220ms var(--ease-out),
    transform 320ms var(--ease-out);
}
.stash-swap-enter-from {
  opacity: 0;
  transform: translateY(12px) scale(0.985);
}
.stash-swap-leave-active {
  transition:
    opacity 140ms var(--ease-out),
    transform 160ms var(--ease-out);
}
.stash-swap-leave-to {
  opacity: 0;
  transform: translateY(-8px) scale(0.99);
}
.text-field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  color: var(--ink-2);
  font-size: 11.5px;
  font-weight: 600;
}
.text-field-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
}
.text-field-label {
  color: var(--ink-2);
}
.text-clip-btn {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  border: 1px solid var(--line-strong);
  background: transparent;
  color: var(--ink-2);
  border-radius: 7px;
  padding: 2px 8px;
  font-size: 11px;
  font-weight: 550;
  cursor: pointer;
  font-family: inherit;
  transition:
    background 120ms ease,
    color 120ms ease,
    border-color 120ms ease;
}
.text-clip-btn:hover:not(:disabled) {
  background: var(--surface-2);
  border-color: var(--accent);
  color: var(--ink);
}
.text-clip-btn:disabled {
  opacity: 0.58;
  cursor: wait;
}
.text-field input,
.text-field textarea {
  width: 100%;
  border: 1px solid var(--line-strong);
  border-radius: 10px;
  padding: 8px 10px;
  font-size: 13px;
  font-family: inherit;
  background: var(--surface-raised);
  color: var(--ink);
  outline: none;
  line-height: 1.6;
  box-sizing: border-box;
}
.text-field textarea {
  resize: none;
  padding: 10px 12px;
}
.text-field input:focus,
.text-field textarea:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.text-actions {
  display: flex;
  gap: 8px;
}
.act {
  border: 1px solid var(--line-strong);
  background: var(--surface-raised);
  color: var(--ink);
  border-radius: 9px;
  padding: 7px 14px;
  font-size: 12.5px;
  font-weight: 550;
  cursor: pointer;
  font-family: inherit;
}
.act.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--on-accent);
}
.act.ghost {
  border-color: transparent;
  color: var(--ink-2);
}

.toast {
  position: absolute;
  bottom: 52px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--ink);
  color: var(--surface);
  font-size: 12px;
  padding: 7px 14px;
  border-radius: 999px;
  box-shadow: var(--shadow-pop);
  white-space: nowrap;
  pointer-events: none;
}
.toast-enter-active,
.toast-leave-active {
  transition:
    opacity 180ms ease,
    transform 240ms var(--ease-out);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(6px);
}

/* 内嵌降级菜单：覆盖浮动面板窗口的可视区域，位置由 inlineMenuStyle 收敛 */
.inline-menu-layer {
  position: fixed;
  inset: 0;
  z-index: 60;
}
.inline-menu-pos {
  position: absolute;
}
</style>
