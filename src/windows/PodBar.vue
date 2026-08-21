<script setup lang="ts">
/**
 * 单个「匣」的胶囊条：贴在屏幕边缘的圆形短条。
 * - 背景全透明，短条半透明填充
 * - 悬停 -> 弹出该匣面板；移出 -> 面板收回（看门狗）
 * - 拖入文件时短条变为圆角矩形主动接纳，松手后弹出面板
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ipc } from "@/lib/ipc";
import { Events, listenCurrent } from "@/lib/events";
import { springValue, type SpringHandle } from "@/lib/spring";
import { useSettingsStore } from "@/stores/settings";
import { useStagingStore } from "@/stores/staging";
import type { DropAction, ModifierState } from "@/types";

const props = defineProps<{ podId: number }>();
const settingsStore = useSettingsStore();
const staging = useStagingStore();

const pod = computed(() => settingsStore.pod(props.podId));
const vertical = computed(() => pod.value?.edge === "left" || pod.value?.edge === "right");
const hovering = ref(false);
const accepting = ref(false);
let hoverTimeout: number | undefined;
let shortSpring: SpringHandle | null = null;
const short = ref(44);
const unlisteners: Array<() => void> = [];
let mounted = true;
let modifierSnapshot: ModifierState | null = null;
let modifierSample: Promise<ModifierState> | null = null;
let modifierSampleSeq = 0;
let lastModifierSample = 0;

type ConcreteDropAction = Exclude<DropAction, "ask">;
const NO_MODIFIERS: ModifierState = { ctrl: false, shift: false, alt: false };

const count = computed(
  () => staging.items.filter((i) => i.podId === props.podId).length,
);

const capsuleStyle = computed(() =>
  vertical.value
    ? { width: short.value + "px", height: "100%", opacity: pod.value?.opacity ?? 0.85 }
    : { height: short.value + "px", width: "100%", opacity: pod.value?.opacity ?? 0.85 },
);

/* 仅在拖入接纳时短条变宽（圆角矩形）；悬停弹出面板不改变形状。
   目标 62 与 Rust 侧 POD_BAR_ACCEPT 一致：胶囊填满窗口，圆角矩形完整显示。 */
watch(accepting, (v) => shortSpring?.setTarget(v ? 62 : 44));

function clearHoverTimer() {
  window.clearTimeout(hoverTimeout);
  hoverTimeout = undefined;
}

function retainUnlistener(unlisten: () => void) {
  if (mounted) unlisteners.push(unlisten);
  else unlisten();
}

function reportPresence(inside: boolean) {
  void ipc
    .reportPresence(props.podId, "bar", inside)
    .catch((err) => console.error("bar presence update failed", err));
}

function setAccept(accept: boolean) {
  void ipc
    .setPodAccept(props.podId, accept)
    .catch((err) => console.error("drop accept update failed", err));
}

function showPanel() {
  void ipc.showPanel(props.podId).catch((err) => console.error("show panel failed", err));
}

function sampleModifiers(force = false) {
  const now = performance.now();
  if (!force && now - lastModifierSample < 32) return;
  lastModifierSample = now;
  const sequence = ++modifierSampleSeq;
  const request = ipc.getModifierState().catch(() => NO_MODIFIERS);
  modifierSample = request;
  void request.then((value) => {
    if (sequence === modifierSampleSeq) modifierSnapshot = value;
  });
}

function onPointerEnter() {
  hovering.value = true;
  clearHoverTimer();
  reportPresence(true);
  hoverTimeout = window.setTimeout(() => {
    hoverTimeout = undefined;
    if (hovering.value && !accepting.value && pod.value?.enabled) {
      showPanel();
    }
  }, pod.value?.hoverDelayMs ?? 120);
}

function onPointerLeave() {
  hovering.value = false;
  clearHoverTimer();
  reportPresence(false);
}

function onClick() {
  // A pending hover callback must not reopen a panel that this click just hid.
  clearHoverTimer();
  void ipc.togglePanel(props.podId).catch((err) => console.error("toggle panel failed", err));
}

/* ---- 文件拖入（原生） ---- */
async function handleDrop(paths: string[], sampled: Promise<ModifierState> | null) {
  accepting.value = false;
  setAccept(false);
  if (!pod.value || paths.length === 0) return;
  const action = pod.value.dropAction ?? "ask";
  // Prefer the last sample requested while the native drag was still over the
  // bar. Reading only after drop races with the user releasing Ctrl/Shift/Alt.
  const mods = sampled ? await sampled : (modifierSnapshot ?? NO_MODIFIERS);
  let chosen: ConcreteDropAction | null = null;
  if (mods.ctrl) chosen = "copy";
  else if (mods.shift) chosen = "move";
  else if (mods.alt) chosen = "shortcut";
  else if (action !== "ask") chosen = action as ConcreteDropAction;

  try {
    if (chosen) {
      const result = await ipc.stagePaths(props.podId, paths, chosen);
      if (result.warnings.length) {
        const warning = result.warnings[0];
        console.warn("stage completed with source cleanup warning", result.warnings);
        void ipc.logFrontend(`暂存警告 ${warning.name}: ${warning.error}`).catch(() => {});
      }
      await staging.refresh(props.podId).catch((err) => {
        console.error("post-stage refresh failed", err);
      });
    } else {
      await ipc.holdPendingDrop(props.podId, paths);
    }
    // 拖入完成后弹出面板
    showPanel();
  } catch (err) {
    console.error("stage failed", err);
  }
}

/* ---- 文字拖入（HTML5 兜底；原生只拦文件） ---- */
async function onHtmlDrop(e: DragEvent) {
  const dt = e.dataTransfer;
  if (!dt || dt.files?.length) return;
  const text = dt.getData("text/plain");
  if (text && pod.value) {
    e.preventDefault();
    try {
      await ipc.stageText(props.podId, text);
      await staging.refresh(props.podId).catch((err) => {
        console.error("post-text-stage refresh failed", err);
      });
      showPanel();
    } catch (err) {
      console.error("stage text failed", err);
    }
  }
}

onMounted(async () => {
  /* 胶囊短边弹簧要先就绪，避免拖放事件早于初始化完成。 */
  shortSpring = springValue(44, 44, (v) => (short.value = v), {
    response: 0.32,
    damping: 1,
  });

  try {
    await settingsStore
      .listenChanges()
      .catch((err) => console.error("settings listener failed", err));
    await staging
      .listenChanges(props.podId)
      .then(retainUnlistener)
      .catch((err) => console.error("items listener failed", err));
    await settingsStore.load();
    if (!mounted) return;
    staging.setActivePod(props.podId);
    await staging.refresh(props.podId);
    if (!mounted) return;
  } catch (err) {
    console.error("pod bar initialization failed", err);
    return;
  }

  try {
    /* 原生拖放事件（文件路径） */
    if (ipc.inTauri) {
      const { getCurrentWebview } = await import("@tauri-apps/api/webview");
      retainUnlistener(await getCurrentWebview().onDragDropEvent((event) => {
        const p = event.payload;
        if (p.type === "enter") {
          clearHoverTimer();
          accepting.value = true;
          modifierSnapshot = null;
          sampleModifiers(true);
          setAccept(true);
        } else if (p.type === "over") {
          accepting.value = true;
          sampleModifiers();
        } else if (p.type === "leave") {
          accepting.value = false;
          modifierSampleSeq += 1;
          modifierSnapshot = null;
          modifierSample = null;
          setAccept(false);
        } else if (p.type === "drop") {
          const sampled = modifierSample;
          accepting.value = false;
          setAccept(false);
          modifierSampleSeq += 1;
          modifierSnapshot = null;
          modifierSample = null;
          void handleDrop(p.paths, sampled);
        }
      }));
    }

    /* 剪贴板收集热键：只由本匣处理（事件携带 podId） */
    retainUnlistener(await listenCurrent<{ podId?: number }>(Events.CollectClipboard, async (p) => {
      if (!p || (p.podId && p.podId !== props.podId)) return;
      if (!pod.value) return;
      try {
        const { readText } = await import("@tauri-apps/plugin-clipboard-manager");
        const text = await readText();
        if (text.trim()) {
          await ipc.stageText(props.podId, text);
          await staging.refresh(props.podId).catch((err) => {
            console.error("post-clipboard-stage refresh failed", err);
          });
        }
      } catch (err) {
        console.error("collect clipboard failed", err);
      }
    }));
  } catch (err) {
    console.error("pod bar event initialization failed", err);
  }
});

onBeforeUnmount(() => {
  mounted = false;
  clearHoverTimer();
  shortSpring?.stop();
  modifierSampleSeq += 1;
  modifierSnapshot = null;
  modifierSample = null;
  setAccept(false);
  reportPresence(false);
  unlisteners.splice(0).forEach((unlisten) => unlisten());
});
</script>

<template>
  <div
    class="bar-root"
    :class="`edge-${pod?.edge ?? 'left'}`"
    role="button"
    tabindex="0"
    :aria-label="`打开${pod?.name ?? '匣'}面板`"
    @pointerenter="onPointerEnter"
    @pointerleave="onPointerLeave"
    @click="onClick"
    @keydown.enter.prevent="onClick"
    @keydown.space.prevent="onClick"
    @dragover.prevent
    @drop="onHtmlDrop"
  >
    <div class="capsule" :class="{ accepting, hovering }" :style="capsuleStyle">
      <div class="capsule-inner">
        <Transition name="fade">
          <div v-if="accepting" class="drop-hint">松手暂存</div>
        </Transition>
        <template v-if="!accepting">
          <div v-if="count > 0" class="count-badge">{{ count > 99 ? "99+" : count }}</div>
          <div v-else class="idle-mark" />
        </template>
      </div>
    </div>
  </div>
</template>

<style scoped>
.bar-root {
  position: fixed;
  inset: 0;
  overflow: hidden;
  cursor: default;
}
.bar-root:focus-visible {
  outline: none;
}
.bar-root:focus-visible .capsule {
  box-shadow: inset 0 0 0 2px var(--accent);
}

.capsule {
  position: absolute;
  min-width: 44px;
  min-height: 44px;
  will-change: width, height;
  background: var(--bar-glass);
  box-shadow: inset 0 0 0 1px var(--glass-line);
  display: flex;
  align-items: center;
  justify-content: center;
  transition: box-shadow 200ms var(--ease-out), background 200ms var(--ease-out);
}
.edge-left .capsule {
  left: 0;
  top: 0;
  bottom: 0;
  border-radius: 0 22px 22px 0;
}
.edge-right .capsule {
  right: 0;
  top: 0;
  bottom: 0;
  border-radius: 22px 0 0 22px;
}
.edge-top .capsule {
  top: 0;
  left: 0;
  right: 0;
  border-radius: 0 0 22px 22px;
}
.edge-bottom .capsule {
  bottom: 0;
  left: 0;
  right: 0;
  border-radius: 22px 22px 0 0;
}

.capsule.hovering {
  background: var(--bar-glass-hover);
}
.capsule.accepting {
  background: var(--accent);
  box-shadow: inset 0 0 0 1.5px oklch(1 0 0 / 0.25);
  animation: breathe 1.1s ease-in-out infinite;
}
@keyframes breathe {
  0%, 100% { filter: brightness(1); }
  50% { filter: brightness(1.18); }
}

.capsule-inner {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 10px;
}
.count-badge {
  min-width: 24px;
  height: 24px;
  padding: 0 6px;
  border-radius: 999px;
  background: var(--accent);
  color: var(--on-accent);
  font-size: 12px;
  font-weight: 600;
  display: flex;
  align-items: center;
  justify-content: center;
  box-shadow: 0 2px 8px rgb(0 0 0 / 0.25);
}
.idle-mark {
  width: 8px;
  height: 8px;
  border-radius: 999px;
  background: var(--ink-3);
}
.drop-hint {
  writing-mode: vertical-rl;
  letter-spacing: 0.3em;
  font-size: 12px;
  font-weight: 650;
  color: var(--on-accent);
  white-space: nowrap;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 150ms ease;
}
.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

@media (prefers-reduced-motion: reduce) {
  .capsule.accepting {
    animation: none;
  }
}
</style>
