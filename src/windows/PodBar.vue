<script setup lang="ts">
/**
 * 单个「匣」的胶囊条：贴在屏幕边缘的圆形短条。
 * - 背景全透明，短条半透明填充（固定普通半透明，无系统材质）
 * - 悬停 -> 弹出该匣面板；移出 -> 面板淡出隐藏（看门狗）
 * - 拖入文件时短条变为圆角矩形主动接纳，松手后弹出面板
 */
import { computed, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { useUnlisteners } from "@/composables/useUnlisteners";
import { dropActionFor } from "@/domain/dropAction";
import { monitorLogicalSpan, offsetAfterDrag } from "@/domain/podPosition";
import type { ModifierState } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";
import { clampOpacity } from "@/lib/format";
import { springValue, type SpringHandle } from "@/lib/spring";
import { useSettingsStore } from "@/stores/settings";
import { useStagingStore } from "@/stores/staging";

const props = defineProps<{ podId: number }>();
const settingsStore = useSettingsStore();
const staging = useStagingStore();

const pod = computed(() => settingsStore.pod(props.podId));
const vertical = computed(() => pod.value?.edge === "left" || pod.value?.edge === "right");
const horizontal = computed(() => pod.value?.edge === "top" || pod.value?.edge === "bottom");
const hovering = ref(false);
const accepting = ref(false);
let hoverTimeout: number | undefined;
let shortSpring: SpringHandle | null = null;
const short = ref(44);
const { retainUnlistener, disposeUnlisteners, isMounted } = useUnlisteners();
let modifierSnapshot: ModifierState | null = null;
let modifierSample: Promise<ModifierState> | null = null;
let modifierSampleSeq = 0;
let lastModifierSample = 0;

const NO_MODIFIERS: ModifierState = { ctrl: false, shift: false, alt: false };

const dragging = ref(false);
// 拖拽刚结束的时间戳；此后一小段时间内的 click 视为拖拽残留，不触发 toggle。
// 用时间戳而非布尔值：pointerup 落在窗口外时不会留下"永久吞点击"的粘滞状态。
let justDraggedAt = 0;
let dragStartPos = 0;
let dragStartOffset = 0;
let dragStartScreenLength = 1080; // 拖动开始时的屏幕尺寸

const count = computed(() => staging.activeItems.length);

/** 胶囊条短边；后端校验范围 28-96，加载前回退默认 44。 */
const barWidth = computed(() => Math.min(96, Math.max(28, pod.value?.barWidth ?? 44)));
/** 拖入接纳态在匣宽度基础上加宽，与 Rust 侧 POD_BAR_ACCEPT_GROW 一致。 */
const acceptWidth = computed(() => barWidth.value + 18);

const capsuleStyle = computed<Record<string, string>>(() => {
  const borderColor = pod.value?.borderColor || "var(--glass-line)";
  const borderOpacity = Math.min(1, Math.max(0, pod.value?.borderOpacity ?? 1));
  const appearance = {
    "--pod-opacity": `${clampOpacity(pod.value?.opacity) * 100}%`,
    "--bar-radius": `${pod.value?.cornerRadius ?? 22}px`,
    "--pod-border": `color-mix(in srgb, ${borderColor} ${borderOpacity * 100}%, transparent)`,
  };
  return vertical.value
    ? { ...appearance, width: short.value + "px", height: "100%", minWidth: barWidth.value + "px" }
    : { ...appearance, height: short.value + "px", width: "100%", minHeight: barWidth.value + "px" };
});

/* 仅在拖入接纳时短条变宽（圆角矩形）；悬停弹出面板不改变形状。
   目标宽度 = 匣宽度 + 18，与 Rust 侧 POD_BAR_ACCEPT_GROW 一致：胶囊填满窗口，
   圆角矩形完整显示。 */
watch(
  [accepting, barWidth],
  ([accept]) => shortSpring?.setTarget(accept ? acceptWidth.value : barWidth.value),
);

function clearHoverTimer() {
  window.clearTimeout(hoverTimeout);
  hoverTimeout = undefined;
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
  // 拖动中不弹出面板
  if (dragging.value) return;
  hoverTimeout = window.setTimeout(() => {
    hoverTimeout = undefined;
    if (
      hovering.value &&
      !accepting.value &&
      pod.value?.enabled &&
      pod.value.hoverOpen !== false
    ) {
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
  // 如果刚完成拖动，不触发点击
  if (performance.now() - justDraggedAt < 300) return;
  // 点击关闭面板后，尚未执行的悬停回调不能把它重新打开。
  clearHoverTimer();
  void ipc.togglePanel(props.podId).catch((err) => console.error("toggle panel failed", err));
}

function onPointerDown(e: PointerEvent) {
  // 只响应主按钮（左键）
  if (e.button !== 0 || !pod.value) return;

  // 记录起始位置和当前 offset
  dragStartPos = vertical.value ? e.screenY : e.screenX;
  dragStartOffset = pod.value.offset;
  dragging.value = false;
  justDraggedAt = 0;

  const selectedMonitor = pod.value.monitor
    ? settingsStore.monitors.find((monitor) => monitor.name === pod.value?.monitor)
    : settingsStore.monitors.find((monitor) => monitor.primary);
  dragStartScreenLength = selectedMonitor
    ? monitorLogicalSpan(selectedMonitor, vertical.value)
    : vertical.value
      ? window.screen.height
      : window.screen.width;

  // 添加全局监听
  window.addEventListener("pointermove", onPointerMove);
  window.addEventListener("pointerup", onPointerUp);
  window.addEventListener("pointercancel", onPointerUp);

  // 阻止默认行为和事件冒泡
  e.preventDefault();
  e.stopPropagation();
}

// 指针移动事件远快于 IPC 往返：在途只保留一个请求，并始终携带最新 offset，
// 既避免请求堆积，也避免先发后至的旧 offset 让条来回跳动。
let moveInFlight = false;
let latestMoveOffset: number | null = null;

function requestMovePodBar(offset: number) {
  latestMoveOffset = offset;
  if (moveInFlight) return;
  moveInFlight = true;
  const pump = () => {
    const current = latestMoveOffset;
    latestMoveOffset = null;
    if (current === null) {
      moveInFlight = false;
      return;
    }
    void ipc
      .movePodBar(props.podId, current)
      .catch((err) => console.error("move pod bar failed", err))
      .finally(() => (latestMoveOffset !== null ? pump() : (moveInFlight = false)));
  };
  pump();
}

function onPointerMove(e: PointerEvent) {
  if (!pod.value) return;

  const currentPos = vertical.value ? e.screenY : e.screenX;
  const delta = currentPos - dragStartPos;

  // 拖动阈值：3px
  if (!dragging.value && Math.abs(delta) < 3) return;

  // 标记为拖动中
  if (!dragging.value) {
    dragging.value = true;
    // 拖动时取消 hover 定时器，避免弹出面板
    window.clearTimeout(hoverTimeout);
    // 隐藏面板（如果已显示）
    void ipc.hidePanel(props.podId);
  }

  // 计算新的 offset（0-1 范围）
  const newOffset = offsetAfterDrag(dragStartOffset, delta, dragStartScreenLength);

  // 使用轻量级命令实时移动窗口（不写数据库，性能更好）
  requestMovePodBar(newOffset);

  // 阻止默认行为
  e.preventDefault();
}

async function onPointerUp(e: PointerEvent) {
  // 移除全局监听
  window.removeEventListener("pointermove", onPointerMove);
  window.removeEventListener("pointerup", onPointerUp);
  window.removeEventListener("pointercancel", onPointerUp);

  if (dragging.value) {
    dragging.value = false;
    justDraggedAt = performance.now(); // 标记刚刚完成拖动，防止触发点击
    
    // 计算最终的 offset
    const currentPos = vertical.value ? e.screenY : e.screenX;
    const delta = currentPos - dragStartPos;
    const finalOffset = offsetAfterDrag(dragStartOffset, delta, dragStartScreenLength);
    
    // 保存到数据库
    if (pod.value) {
      await ipc.updatePod(props.podId, { offset: finalOffset });
    }
    
    // 阻止拖动结束后触发 onClick
    e.preventDefault();
    e.stopPropagation();
  }
}

async function handleDrop(paths: string[], sampled: Promise<ModifierState> | null) {
  accepting.value = false;
  setAccept(false);
  if (!pod.value || paths.length === 0) return;
  const action = pod.value.dropAction ?? "ask";
  // 使用原生拖拽仍位于匣上时的最后一次采样，避免松手后读取不到修饰键。
  const mods = sampled ? await sampled : (modifierSnapshot ?? NO_MODIFIERS);
  const chosen = dropActionFor(mods, action);

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

async function onHtmlDrop(e: DragEvent) {
  const dt = e.dataTransfer;
  if (!dt || dt.files?.length) return;
  const text = dt.getData("text/plain");
  // 文本拖放（包括空文本）都必须阻止 WebView 默认行为（如导航到拖放内容）。
  e.preventDefault();
  if (!text || !pod.value) return;
  try {
    await staging.stageTextAndRefresh(props.podId, text);
    showPanel();
  } catch (err) {
    console.error("stage text failed", err);
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
    if (!isMounted()) return;
    staging.setActivePod(props.podId);
    await staging.refresh(props.podId);
    if (!isMounted()) return;
  } catch (err) {
    console.error("pod bar initialization failed", err);
    return;
  }

  try {
    /* 原生拖放事件（文件路径） */
    if (ipc.inTauri) {
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
        const text = await readText();
        if (text.trim()) {
          await staging.stageTextAndRefresh(props.podId, text);
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
  disposeUnlisteners();
  clearHoverTimer();
  shortSpring?.stop();
  modifierSampleSeq += 1;
  modifierSnapshot = null;
  modifierSample = null;
  setAccept(false);
  reportPresence(false);
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
    @pointerdown="onPointerDown"
    @click="onClick"
    @keydown.enter.prevent="onClick"
    @keydown.space.prevent="onClick"
    @dragover.prevent
    @drop="onHtmlDrop"
  >
    <div class="capsule" :class="{ accepting, hovering, dragging }" :style="capsuleStyle">
      <div class="capsule-inner">
        <Transition name="fade">
          <div v-if="accepting" class="drop-hint" :class="{ 'drop-hint-horizontal': horizontal }">松手暂存</div>
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
  will-change: width, height;
  background: color-mix(
    in srgb,
    var(--surface) var(--pod-opacity, 100%),
    transparent
  );
  box-shadow: inset 0 0 0 1px var(--pod-border, var(--glass-line));
  display: flex;
  align-items: center;
  justify-content: center;
  transition: box-shadow 200ms var(--ease-out), background 200ms var(--ease-out);
}
.edge-left .capsule {
  left: 0;
  top: 0;
  bottom: 0;
  border-radius: 0 var(--bar-radius, 22px) var(--bar-radius, 22px) 0;
}
.edge-right .capsule {
  right: 0;
  top: 0;
  bottom: 0;
  border-radius: var(--bar-radius, 22px) 0 0 var(--bar-radius, 22px);
}
.edge-top .capsule {
  top: 0;
  left: 0;
  right: 0;
  border-radius: 0 0 var(--bar-radius, 22px) var(--bar-radius, 22px);
}
.edge-bottom .capsule {
  bottom: 0;
  left: 0;
  right: 0;
  border-radius: var(--bar-radius, 22px) var(--bar-radius, 22px) 0 0;
}

.capsule.hovering {
  background: color-mix(
    in srgb,
    var(--surface-raised) var(--pod-opacity, 100%),
    transparent
  );
}
.capsule.accepting {
  background: var(--accent);
  box-shadow: inset 0 0 0 1.5px oklch(1 0 0 / 0.25);
  animation: breathe 1.1s ease-in-out infinite;
}
.capsule.dragging {
  background: var(--accent);
  box-shadow: inset 0 0 0 1.5px oklch(1 0 0 / 0.25);
  cursor: grabbing;
  transition: none;
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
.drop-hint-horizontal {
  writing-mode: horizontal-tb;
  letter-spacing: 0.2em;
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
