<script setup lang="ts">
/**
 * 单个「匣」的弹出面板：列表 / 拖入询问 / 冲突解决 三种模式。
 * 不抢焦点显示（Rust 侧 SW_SHOWNOACTIVATE），面板材质恒定全量下发、
 * 不随焦点降级；指针离开超时后淡出隐藏（Rust 看门狗），
 * 重新悬停或主动弹出时淡入。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, ref, watch } from "vue";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { readText } from "@tauri-apps/plugin-clipboard-manager";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { useUnlisteners } from "@/composables/useUnlisteners";
import { useToast } from "@/composables/useToast";
import { exportVerb, presentExport } from "@/domain/exportPresentation";
import { updateSelection, type SelectionMode } from "@/domain/selection";
import type {
  ConflictStrategy,
  DragCutToken,
  DropAction,
  ExportMode,
  ExportResult,
  OperationPreview,
  PanelMode,
  PanelState,
  SecurityStatus,
  StagedItem,
} from "@/domain/types";
import { buildItemMenu, type MenuItemSpec } from "@/domain/menu";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";
import { BROWSER_PREVIEW_EXPORT_ROOT } from "@/lib/env";
import { clampOpacity } from "@/lib/format";
import { useSettingsStore } from "@/stores/settings";
import { useStagingStore } from "@/stores/staging";
import ItemRow from "@/components/ItemRow.vue";
import ActionChooser from "@/components/ActionChooser.vue";
import ConflictDialog from "@/components/ConflictDialog.vue";
import ContextMenu from "@/components/ContextMenu.vue";
import SegmentedControl from "@/components/SegmentedControl.vue";
import TrustExportDialog from "@/components/TrustExportDialog.vue";

const props = defineProps<{ podId: number }>();

const settingsStore = useSettingsStore();
const staging = useStagingStore();

const pod = computed(() => settingsStore.pod(props.podId));
const panelStyle = computed<Record<string, string>>(() => {
  // 面板填充色与不透明度独立于胶囊条；旧配置缺失时回退匣的不透明度。
  const opacity = clampOpacity(pod.value?.panelOpacity ?? pod.value?.opacity);
  const fill = pod.value?.panelColor?.trim() || "var(--surface)";
  return { "--pod-opacity": `${opacity * 100}%`, "--pod-fill": fill };
});

const mode = ref<PanelMode>("list");
const pendingPaths = ref<string[]>([]);
const dragMode = ref<"copy" | "move">("copy");
const textOpen = ref(false);
const textTitle = ref("");
const textValue = ref("");
const trustOpen = ref(false);
const securityStatus = ref<SecurityStatus | null>(null);
const unlocking = ref(false);
let securityTimer: number | undefined;
let anchorId: number | null = null;
let confirmClearTimer: number | undefined;
const { retainUnlistener, disposeUnlisteners, isMounted } = useUnlisteners();

const pinned = ref(false);
const pinBusy = ref(false);
const exportBusy = ref(false);
const listActionBusy = ref(false);
const dragBusy = ref(false);
const askBusy = ref(false);
const textBusy = ref(false);
/** 导出 / 删除 / 拖出都会按启动时的选择或文件状态处理；三者任一进行中即视为忙碌。 */
const anyBusy = computed(() => exportBusy.value || listActionBusy.value || dragBusy.value);
const rootEl = ref<HTMLElement | null>(null);
const headEl = ref<HTMLElement | null>(null);
const listEl = ref<HTMLElement | null>(null);
const contentEl = ref<HTMLElement | null>(null);
const footEl = ref<HTMLElement | null>(null);
let lastFadeIn = Number.NEGATIVE_INFINITY;
let modeRevision = 0;
let pinRevision = 0;
const { toast, showToast, disposeToast } = useToast(2200, isMounted);

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
  pinRevision += 1;
  pinBusy.value = true;
  pinned.value = next;
  try {
    await ipc.setPanelPinned(props.podId, next);
  } catch (err) {
    pinned.value = previous;
    console.error("pin update failed", err);
    showToast("固定状态更新失败，请重试");
  } finally {
    pinBusy.value = false;
  }
}

const conflict = ref<{ names: string[]; ids: number[]; dest: string; mode: ExportMode } | null>(
  null,
);

const items = computed(() => staging.activeItems);
const sensitiveLocked = computed(
  () => Boolean(pod.value?.security.enabled && securityStatus.value?.locked !== false),
);
const selectedItems = computed(() => staging.selectedItems);
const selectedCount = computed(() => selectedItems.value.length);

function currentSelectedIds(): number[] {
  return selectedItems.value.map((item) => item.id);
}

function clearSelection() {
  staging.clearSelection();
  anchorId = null;
}

function selectAll() {
  staging.selectAll();
  anchorId = items.value[0]?.id ?? null;
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

function onSelect(id: number, mode: SelectionMode) {
  // 导出、拖出和删除会按启动时的选择处理；执行期间锁定选择，避免完成时覆盖新选择。
  if (anyBusy.value) return;
  const next = updateSelection(
    staging.selectedIds,
    items.value.map((item) => item.id),
    id,
    mode,
    anchorId,
  );
  staging.selectedIds = next.selected;
  anchorId = next.anchor;
}

watch(
  () => items.value.map((item) => item.id).join(","),
  () => {
    if (anchorId != null && !items.value.some((item) => item.id === anchorId)) {
      anchorId = null;
    }
  },
);

function selectedOrSingle(item: StagedItem): string[] {
  if (staging.selectedIds.has(item.id)) {
    return staging.selectedItems.map((i) => i.stagingPath);
  }
  return [item.stagingPath];
}

function makeDragIcon(paths: string[], ext: string | null): string {
  const c = document.createElement("canvas");
  const dpr = window.devicePixelRatio || 1;
  const w = 44;
  c.width = 64 * dpr;
  c.height = 64 * dpr;
  const ctx = c.getContext("2d");
  if (!ctx) return "";
  ctx.scale(dpr, dpr);
  const css = getComputedStyle(document.documentElement);
  const color = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
  ctx.fillStyle = color("--surface-raised", "#f6f7f8");
  const r = 10;
  roundRect(ctx, 10, 8, w, 48, r);
  ctx.fill();
  ctx.strokeStyle = color("--line-strong", "#d6dade");
  ctx.lineWidth = 1.5;
  roundRect(ctx, 10, 8, w, 48, r);
  ctx.stroke();
  ctx.fillStyle = color("--ink", "#3d434a");
  ctx.font = "600 13px 'Segoe UI', sans-serif";
  ctx.textAlign = "center";
  ctx.fillText((ext ?? "文件").slice(0, 5).toUpperCase(), 32, 36);
  if (paths.length > 1) {
    ctx.fillStyle = color("--accent", "#2d7ca3");
    ctx.beginPath();
    ctx.arc(46, 44, 11, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = "#fff";
    ctx.font = "600 11px 'Segoe UI', sans-serif";
    ctx.fillText(String(paths.length), 46, 48);
  }
  return c.toDataURL("image/png");
}

function roundRect(ctx: CanvasRenderingContext2D, x: number, y: number, w: number, h: number, r: number) {
  ctx.beginPath();
  ctx.moveTo(x + r, y);
  ctx.arcTo(x + w, y, x + w, y + h, r);
  ctx.arcTo(x + w, y + h, x, y + h, r);
  ctx.arcTo(x, y + h, x, y, r);
  ctx.arcTo(x, y, x + w, y, r);
  ctx.closePath();
}

async function onDragOut(paths: string[]) {
  if (paths.length === 0 || anyBusy.value) return;
  const first = staging.items.find((i) => i.stagingPath === paths[0]);
  const icon = makeDragIcon(paths, first?.ext ?? null);
  // 准备拖出时固定模式快照，确保 OLE 效果与是否清理源文件使用同一模式。
  const requestedMode = dragMode.value;
  const isCut = requestedMode === "move";
  let cutToken: DragCutToken | null = null;
  dragBusy.value = true;
  try {
    await ipc.setDraggingOut(props.podId, true);
    if (isCut) cutToken = await ipc.prepareDragCut(props.podId, paths);
    const dropped = await ipc.startDragOut(paths, icon, requestedMode);
    if (dropped && isCut) {
      try {
        if (!cutToken) throw new Error("剪切令牌缺失");
        await ipc.finalizeDragCut(cutToken);
        cutToken = null;
      } catch (err) {
        console.error("drag destination accepted but source cleanup failed", err);
        await staging.refresh(props.podId).catch((refreshError) => {
          console.error("post-drag failure refresh failed", refreshError);
        });
        showToast("目标已接收文件，但剪切源清理失败");
        return;
      }
      clearSelection();
      if (await refreshAfterMutation("剪切移出")) showToast(`已剪切移出 ${paths.length} 项`);
    }
  } catch (err) {
    console.error("drag out failed", err);
    showToast("拖出失败，请重试");
  } finally {
    if (cutToken) {
      await ipc.cancelDragCut(cutToken).catch((err) => {
        console.error("drag cut token cleanup failed", err);
      });
    }
    await ipc.setDraggingOut(props.podId, false).catch((err) => {
      console.error("drag state cleanup failed", err);
    });
    dragBusy.value = false;
  }
}

async function openItem(item: StagedItem) {
  try {
    // 打开动作走后端校验命令：路径按条目 id 重新解析，WebView 无法打开任意路径。
    await ipc.openStagedItem(item.id);
  } catch (err) {
    console.error("open item failed", err);
    showToast("无法打开此项目");
  }
}

async function revealItem(item: StagedItem) {
  try {
    await revealItemInDir(item.stagingPath);
  } catch (err) {
    console.error("reveal item failed", err);
    showToast("无法打开所在位置");
  }
}

async function removeItem(item: StagedItem) {
  if (anyBusy.value) return;
  listActionBusy.value = true;
  try {
    const preview = await ipc.previewRemoveItems([item.id], true);
    if (!(await confirmPreview(preview))) return;
    await staging.removeItems([item.id], true);
    if (anchorId === item.id) anchorId = null;
    showToast("已移出暂存，24 小时内可在安心中心恢复");
  } catch (err) {
    console.error("remove item failed", err);
    showToast("移出失败，请重试");
  } finally {
    listActionBusy.value = false;
  }
}

async function removeSelected() {
  if (!selectedCount.value || anyBusy.value) return;
  const n = selectedCount.value;
  listActionBusy.value = true;
  try {
    const ids = currentSelectedIds();
    const preview = await ipc.previewRemoveItems(ids, true);
    if (!(await confirmPreview(preview))) return;
    await staging.removeItems(ids, true);
    anchorId = null;
    showToast(`已移出 ${n} 项，24 小时内可恢复`);
  } catch (err) {
    console.error("remove selected failed", err);
    showToast("移出失败，请重试");
  } finally {
    listActionBusy.value = false;
  }
}

async function pickDest(): Promise<string | null> {
  if (!ipc.inTauri) return BROWSER_PREVIEW_EXPORT_ROOT;
  const dir = await open({ directory: true, multiple: false, title: "选择目标文件夹" });
  return typeof dir === "string" ? dir : null;
}

async function stageAccessiblePaths(paths: string[]) {
  if (!paths.length || !pod.value || anyBusy.value) return;
  listActionBusy.value = true;
  try {
    const configured = pod.value.dropAction;
    const action = configured === "ask"
      ? settingsStore.settings?.accessibility.enabled
        ? "copy"
        : null
      : configured;
    if (!action) {
      await ipc.holdPendingDrop(props.podId, paths);
      return;
    }
    const result = await ipc.stagePaths(props.podId, paths, action);
    await refreshAfterMutation("暂存");
    showToast(
      result.warnings.length
        ? `已暂存，另有 ${result.warnings.length} 条提醒`
        : `已${action === "move" ? "移动" : action === "shortcut" ? "创建快捷方式" : "复制"} ${result.items.length} 项`,
    );
  } catch (error) {
    console.error("accessible stage failed", error);
    showToast(`暂存失败：${String(error)}`);
  } finally {
    listActionBusy.value = false;
  }
}

async function pickPaths(directory: boolean) {
  if (anyBusy.value) return;
  if (!ipc.inTauri) {
    await stageAccessiblePaths([directory ? "D:\\示例文件夹" : "D:\\示例文件.txt"]);
    return;
  }
  await ipc.setDraggingOut(props.podId, true).catch(() => {});
  try {
    const selected = await open({
      directory,
      multiple: !directory,
      title: directory ? "选择要暂存的文件夹" : "选择要暂存的文件",
    });
    const paths = Array.isArray(selected) ? selected : typeof selected === "string" ? [selected] : [];
    await stageAccessiblePaths(paths);
  } finally {
    await ipc.setDraggingOut(props.podId, false).catch(() => {});
  }
}

async function pasteClipboardFiles() {
  if (anyBusy.value) return;
  try {
    const paths = await ipc.readClipboardFiles();
    if (!paths.length) {
      showToast("剪贴板里没有文件；请先在资源管理器中复制文件");
      return;
    }
    await stageAccessiblePaths(paths);
  } catch (error) {
    showToast(`无法读取剪贴板文件：${String(error)}`);
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

async function refreshSecurityStatus() {
  if (!pod.value?.security.enabled && !pod.value?.rules.expireDays) {
    securityStatus.value = null;
    return;
  }
  try {
    securityStatus.value = await ipc.getPodSecurityStatus(props.podId);
  } catch (error) {
    console.error("security status failed", error);
  }
}

async function unlockSensitivePod() {
  if (unlocking.value) return;
  unlocking.value = true;
  try {
    securityStatus.value = await ipc.unlockSensitivePod(props.podId);
    await staging.refresh(props.podId);
    showToast("敏感匣已解锁");
  } catch (error) {
    showToast(`解锁失败：${String(error)}`);
  } finally {
    unlocking.value = false;
  }
}

async function lockSensitivePod() {
  await ipc.lockSensitivePod(props.podId);
  securityStatus.value = securityStatus.value
    ? { ...securityStatus.value, locked: true }
    : null;
}

async function confirmPreview(preview: OperationPreview): Promise<boolean> {
  if (!preview.requiresConfirmation) return true;
  const lines = [
    preview.title,
    ...preview.warnings.map((warning) => `注意：${warning}`),
    ...preview.details.slice(0, 6),
  ];
  if (preview.details.length > 6) lines.push(`另有 ${preview.details.length - 6} 项…`);
  const message = lines.join("\n\n");
  if (!ipc.inTauri) return window.confirm(message);
  return ask(message, { title: "操作前预览", kind: "warning" });
}

async function applyExportResult(result: ExportResult, exportMode: ExportMode) {
  const verb = exportVerb(exportMode);
  const presentation = presentExport(result, exportMode);
  if (presentation.selection !== null) {
    staging.setSelection(presentation.selection);
    anchorId = null;
  }
  const refreshed = await refreshAfterMutation(verb);
  let message = presentation.message;
  if (!refreshed) message += "；列表刷新失败";
  showToast(message);
}

async function exportSelected(exportMode: ExportMode) {
  const ids = currentSelectedIds();
  if (!ids.length || anyBusy.value) return;
  exportBusy.value = true;
  try {
    // 原生目录选择器会让指针离开 WebView；操作期间保持面板可见。
    await ipc.setDraggingOut(props.podId, true);
    const dest = await pickDest();
    if (!dest) return;
    const preview = await ipc.previewExportItems(ids, dest, exportMode);
    if (!(await confirmPreview(preview))) return;
    const result = await staging.exportItems(ids, dest, exportMode);
    if (result.conflicts.length > 0) {
      conflict.value = { names: result.conflicts, ids, dest, mode: exportMode };
      mode.value = "conflict";
      await ipc.setPanelMode(props.podId, "conflict").catch((err) => {
        console.error("conflict mode sync failed", err);
        showToast("冲突状态同步失败，请尽快选择处理方式");
      });
      return;
    }
    await applyExportResult(result, exportMode);
  } catch (err) {
    console.error(err);
    if (mode.value === "conflict") {
      conflict.value = null;
      mode.value = "list";
      await ipc.setPanelMode(props.podId, "list").catch(() => {});
    }
    showToast("导出失败，请重试");
  } finally {
    await ipc.setDraggingOut(props.podId, false).catch((err) => {
      console.error("export guard cleanup failed", err);
    });
    exportBusy.value = false;
  }
}

async function resolveConflict(strategy: Exclude<ConflictStrategy, "ask">) {
  const ctx = conflict.value;
  if (!ctx || exportBusy.value) return;
  exportBusy.value = true;
  try {
    const result = await ipc.exportItems(ctx.ids, ctx.dest, ctx.mode, strategy);
    conflict.value = null;
    mode.value = "list";
    const modeSynced = await ipc.setPanelMode(props.podId, "list").then(
      () => true,
      (err) => {
        console.error("conflict completion mode sync failed", err);
        return false;
      },
    );
    await applyExportResult(result, ctx.mode);
    if (!modeSynced) showToast("导出已处理，但面板状态同步失败");
  } catch (err) {
    console.error("resolve conflict failed", err);
    showToast("导出失败，请重试");
  } finally {
    exportBusy.value = false;
  }
}

async function cancelConflict() {
  if (exportBusy.value) return;
  conflict.value = null;
  mode.value = "list";
  await ipc.setPanelMode(props.podId, "list").catch((err) => {
    console.error("cancel conflict failed", err);
  });
}

async function chooseAction(action: DropAction, remember: boolean) {
  if (askBusy.value) return;
  const paths = [...pendingPaths.value];
  if (!paths.length || action === "ask") return;
  askBusy.value = true;
  try {
    const result = await ipc.stagePaths(props.podId, paths, action);
    const verb = action === "shortcut" ? "快捷方式" : exportVerb(action);
    pendingPaths.value = [];
    mode.value = "list";
    const modeSynced = await ipc.setPanelMode(props.podId, "list").then(
      () => true,
      (err) => {
        console.error("pending drop completion mode sync failed", err);
        return false;
      },
    );
    const refreshed = await refreshAfterMutation("暂存");
    if (!modeSynced) showToast("文件已暂存，但面板状态同步失败");
    else if (result.warnings.length) {
      const warning = result.warnings[0];
      showToast(`已暂存，但 ${warning.name} 的源清理需检查：${warning.error}`);
    } else if (refreshed) showToast(`已暂存 ${paths.length} 项（${verb}）`);
    if (remember) {
      try {
        await ipc.updatePod(props.podId, { dropAction: action });
      } catch (err) {
        console.error("remember drop action failed", err);
        showToast("文件已暂存，但默认动作保存失败");
      }
    }
  } catch (err) {
    console.error(err);
    showToast("暂存失败，请重试");
  } finally {
    askBusy.value = false;
  }
}

async function cancelAsk() {
  if (askBusy.value) return;
  pendingPaths.value = [];
  mode.value = "list";
  await ipc.setPanelMode(props.podId, "list").catch((err) => {
    console.error("cancel pending drop failed", err);
    showToast("取消失败，请重试");
  });
}

async function stashText() {
  const content = textValue.value.trim();
  if (!content || textBusy.value) return;
  textBusy.value = true;
  try {
    // 标题留空时自动取正文第一个非空行的前 10 个字。
    const title = textTitle.value.trim() || autoTextTitle(content);
    await ipc.stageText(props.podId, textValue.value, title || undefined);
    textTitle.value = "";
    textValue.value = "";
    textOpen.value = false;
    if (await refreshAfterMutation("文字暂存")) showToast("文字已暂存");
  } catch {
    showToast("暂存失败，请重试");
  } finally {
    textBusy.value = false;
  }
}

function autoTextTitle(content: string): string {
  const firstLine = content
    .split(/\r?\n/)
    .map((line) => line.trim())
    .find((line) => line.length > 0);
  return (firstLine ?? "").slice(0, 10);
}

/** 一键读取剪贴板填入正文：剪贴板有内容时追加，避免覆盖已输入的文字。 */
async function pasteClipboard() {
  try {
    const text = await readText();
    if (!text.trim()) {
      showToast("剪贴板里没有文字");
      return;
    }
    textValue.value = textValue.value ? `${textValue.value}\n${text}` : text;
  } catch (err) {
    console.error("read clipboard failed", err);
    showToast("无法读取剪贴板");
  }
}

const confirmClear = ref(false);
async function clearAll() {
  if (anyBusy.value) return;
  if (!confirmClear.value) {
    confirmClear.value = true;
    window.clearTimeout(confirmClearTimer);
    confirmClearTimer = window.setTimeout(() => (confirmClear.value = false), 2500);
    return;
  }
  window.clearTimeout(confirmClearTimer);
  confirmClear.value = false;
  listActionBusy.value = true;
  try {
    await staging.clearActivePod(true);
    anchorId = null;
    showToast("已清空（文件进回收站）");
  } catch (err) {
    console.error("clear pod failed", err);
    showToast("清空失败，请重试");
  } finally {
    listActionBusy.value = false;
  }
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
  } else if (e.key === "Delete" && mode.value === "list" && !textOpen.value && selectedCount.value) {
    void removeSelected();
  }
}

let ro: ResizeObserver | null = null;
let sizeTimer: number | undefined;
let resizeSequence = 0;

function cssPixels(value: string): number {
  const parsed = Number.parseFloat(value);
  return Number.isFinite(parsed) ? parsed : 0;
}

function scheduleResize() {
  // 文字暂存视图不参与调高：面板保持打开文字编辑前的尺寸，内容超高时滚动。
  if (textOpen.value) return;
  window.clearTimeout(sizeTimer);
  const sequence = ++resizeSequence;
  sizeTimer = window.setTimeout(async () => {
    await nextTick();
    if (sequence !== resizeSequence || !isMounted()) return;
    const root = rootEl.value;
    const body = listEl.value;
    const content = contentEl.value;
    const head = headEl.value;
    if (!root || !body || !content || !head) return;

    // 只测量内容元素；若测量滚动视口，原生窗口高度会被反复回灌而持续增长。
    const bodyStyle = getComputedStyle(body);
    const rootStyle = getComputedStyle(root);
    const bodyPadding = cssPixels(bodyStyle.paddingTop) + cssPixels(bodyStyle.paddingBottom);
    const rootBorder = cssPixels(rootStyle.borderTopWidth) + cssPixels(rootStyle.borderBottomWidth);
    const intrinsicBody = Math.ceil(content.scrollHeight + bodyPadding);
    const bodyHeight = mode.value === "list" ? Math.min(intrinsicBody, 560) : intrinsicBody;
    const chromeHeight = head.offsetHeight + (footEl.value?.offsetHeight ?? 0) + rootBorder;
    await ipc
      .setPanelSize(props.podId, Math.ceil(bodyHeight + chromeHeight))
      .catch((err) => console.error("panel resize failed", err));
  }, 110);
}

watch(
  () => [
    mode.value,
    items.value.length,
    selectedCount.value,
    pod.value?.panelWidth,
  ],
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
    listenCurrent<{ mode: PanelMode; paths?: string[] }>(Events.PanelMode, (p) => {
      modeRevision += 1;
      applyPanelMode(p.mode, p.paths ?? []);
    }),
    listenCurrent<PanelState>(Events.PanelState, (state) => {
      modeRevision += 1;
      pinRevision += 1;
      applyPanelState(state);
    }),
    /* 面板每次出现都重播淡入动画 */
    listenCurrent<never>(Events.PanelShown, () => playFadeIn()),
    /* 固定状态同步 */
    listenCurrent<{ pinned: boolean }>(Events.PanelPinned, (p) => {
      pinRevision += 1;
      pinned.value = p.pinned;
    }),
    /* 右键菜单窗口回传的用户选择 */
    listenCurrent<{ podId: number; action: MenuItemSpec }>(Events.ContextMenuChoice, (p) => {
      if (p.podId !== props.podId) return;
      void runMenuAction(p.action);
    }),
    /* 右键菜单已关闭：解除拖出保活 */
    listenCurrent<{ podId: number }>(Events.ContextMenuClosed, (p) => {
      if (p.podId !== props.podId || !menuOpen.value) return;
      menuOpen.value = false;
      void ipc
        .setDraggingOut(props.podId, false)
        .catch((err) => console.error("menu presence restore failed", err));
    }),
    /* 安心模式 Alt+数字：打开对应面板后直接提供非拖拽文件选择器。 */
    listenCurrent<{ podId: number }>(Events.RequestFilePicker, (p) => {
      if (p.podId !== props.podId) return;
      void pickPaths(false);
    }),
    listenCurrent<{ podId: number; locked: boolean }>(Events.PodLockChanged, (p) => {
      if (p.podId !== props.podId) return;
      if (securityStatus.value) securityStatus.value = { ...securityStatus.value, locked: p.locked };
    }),
    /* 面板开始隐藏：先播放淡出，后端延迟 220ms 再隐藏原生窗口。
       全局临时隐藏也会触发该事件；运行态由其他定向事件同步，不能在此清空询问或冲突。 */
    listenCurrent<never>(Events.PanelHidden, () => {
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
    showToast("面板内容加载失败，请重新打开");
  }
  if (!isMounted()) return;

  window.addEventListener("keydown", onKeydown);
  securityTimer = window.setInterval(refreshSecurityStatus, 30_000);

  ro = new ResizeObserver(() => scheduleResize());
  if (contentEl.value) ro.observe(contentEl.value);

  await nextTick();
  scheduleResize();
  playFadeIn();
});

onBeforeUnmount(() => {
  disposeUnlisteners();
  disposeToast();
  window.removeEventListener("keydown", onKeydown);
  ro?.disconnect();
  window.clearTimeout(confirmClearTimer);
  window.clearTimeout(sizeTimer);
  window.clearInterval(securityTimer);
  resizeSequence += 1;
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

const menuOpen = ref(false);
const inlineMenu = ref<{ items: MenuItemSpec[]; x: number; y: number } | null>(null);

function onItemContextMenu(item: StagedItem, at: { x: number; y: number }) {
  if (anyBusy.value || menuOpen.value) return;
  // 右键未选中的条目：先把选择收敛为该条目，与资源管理器一致。
  if (!staging.selectedIds.has(item.id)) onSelect(item.id, "set");
  const specs = buildItemMenu(staging.selectedItems);
  if (!specs.length) return;
  menuOpen.value = true;
  // 菜单窗口会抢走指针：复用拖出保活语义避免面板被看门狗收起，
  // 菜单关闭后由 CONTEXT_MENU_CLOSED 事件恢复。
  void ipc
    .setDraggingOut(props.podId, true)
    .catch((err) => console.error("menu keep-alive failed", err));
  if (ipc.inTauri) {
    ipc.openContextMenu(props.podId, specs).catch((err) => {
      // 菜单窗口未就绪等异常：降级为面板内渲染，保证右键永远有反馈。
      console.error("menu window unavailable, using inline fallback", err);
      inlineMenu.value = { items: specs, x: at.x, y: at.y };
    });
  } else {
    inlineMenu.value = { items: specs, x: at.x, y: at.y };
  }
}

async function runMenuAction(spec: MenuItemSpec) {
  const ids = spec.itemIds ?? [];
  try {
    switch (spec.id) {
      case "open":
        await ipc.openStagedItem(ids[0]);
        break;
      case "reveal":
        await ipc.revealStagedItems(ids);
        break;
      case "copy": {
        await ipc.copyStagedToClipboard(ids);
        showToast(ids.length > 1 ? `已复制 ${ids.length} 项到剪贴板` : "已复制到剪贴板");
        break;
      }
      case "copyPath":
        await ipc.writeClipboardText(spec.text ?? "");
        showToast("已复制路径");
        break;
      case "remove": {
        await staging.removeItems(ids, true);
        showToast(ids.length > 1 ? `已移出 ${ids.length} 项（文件进回收站）` : "已移出暂存（文件进回收站）");
        break;
      }
    }
  } catch (err) {
    console.error("context menu action failed", err);
    showToast("操作失败，请重试");
  }
}

/** 内嵌降级菜单只在窗口内出现，按窗口边界收敛位置。 */
const inlineMenuStyle = computed(() => {
  if (!inlineMenu.value) return {};
  const x = Math.min(Math.max(4, inlineMenu.value.x), window.innerWidth - 240);
  const y = Math.min(Math.max(4, inlineMenu.value.y), window.innerHeight - 250);
  return { left: `${x}px`, top: `${y}px` };
});

function closeInlineMenu() {
  inlineMenu.value = null;
  if (!menuOpen.value) return;
  menuOpen.value = false;
  void ipc
    .setDraggingOut(props.podId, false)
    .catch((err) => console.error("menu presence restore failed", err));
}

function executeInlineMenu(spec: MenuItemSpec) {
  closeInlineMenu();
  void runMenuAction(spec);
}
</script>

<template>
  <div
    ref="rootEl"
    class="panel-root"
    :style="panelStyle"
    @pointerenter="onPointerEnter"
    @pointerleave="onPointerLeave"
  >
    <header ref="headEl" class="panel-head">
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
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <rect x="5" y="10" width="14" height="10" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" />
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
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
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
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
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
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round">
            <path d="M4 7h16M4 12h10M4 17h7" />
          </svg>
        </button>
        <button
          type="button"
          class="head-btn"
          :class="{ on: pinned }"
          :disabled="pinBusy"
          :aria-pressed="pinned"
          :title="pinned ? '已固定，移开鼠标面板保持展开' : '固定面板（移开鼠标后保持展开）'"
          @click="onTogglePinned"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <path d="M12 17v5" />
            <path d="M9 10.76a2 2 0 0 1-1.11 1.79l-1.78.9A2 2 0 0 0 5 15.24V16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1v-.76a2 2 0 0 0-1.11-1.79l-1.78-.9A2 2 0 0 1 15 10.76V7h1a2 2 0 0 0 0-4H8a2 2 0 0 0 0 4h1z" />
          </svg>
        </button>
        <button type="button" class="head-btn" title="设置" @click="openSettings">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.7 1.7 0 0 0 .34 1.87l.06.06a2 2 0 1 1-2.83 2.83l-.06-.06a1.7 1.7 0 0 0-1.87-.34 1.7 1.7 0 0 0-1 1.55V21a2 2 0 1 1-4 0v-.09a1.7 1.7 0 0 0-1-1.55 1.7 1.7 0 0 0-1.87.34l-.06.06a2 2 0 1 1-2.83-2.83l.06-.06a1.7 1.7 0 0 0 .34-1.87 1.7 1.7 0 0 0-1.55-1H3a2 2 0 1 1 0-4h.09a1.7 1.7 0 0 0 1.55-1 1.7 1.7 0 0 0-.34-1.87l-.06-.06a2 2 0 1 1 2.83-2.83l.06.06a1.7 1.7 0 0 0 1.87.34h.09a1.7 1.7 0 0 0 1-1.55V3a2 2 0 1 1 4 0v.09a1.7 1.7 0 0 0 1 1.55h.09a1.7 1.7 0 0 0 1.87-.34l.06-.06a2 2 0 1 1 2.83 2.83l-.06.06a1.7 1.7 0 0 0-.34 1.87v.09a1.7 1.7 0 0 0 1.55 1H21a2 2 0 1 1 0 4h-.09a1.7 1.7 0 0 0-1.55 1Z" />
          </svg>
        </button>
      </div>
    </header>

    <div ref="listEl" class="panel-body">
      <div ref="contentEl" class="panel-content">
        <section v-if="sensitiveLocked" class="locked-panel" aria-labelledby="locked-title">
          <svg width="34" height="34" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" aria-hidden="true">
            <rect x="4" y="10" width="16" height="11" rx="2" /><path d="M8 10V7a4 4 0 0 1 8 0v3" />
          </svg>
          <h2 id="locked-title">敏感匣已锁定</h2>
          <p>文件由 Windows EFS 在磁盘上保护。通过 Windows Hello 或系统 PIN 后才能查看、搜索、复制或导出。</p>
          <button type="button" class="act primary" :disabled="unlocking" @click="unlockSensitivePod">
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
                    <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true">
                      <rect x="8" y="3" width="8" height="4" rx="1" />
                      <path d="M16 5h2a1 1 0 0 1 1 1v14a1 1 0 0 1-1 1H6a1 1 0 0 1-1-1V6a1 1 0 0 1 1-1h2" />
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
                <button type="button" class="act ghost" :disabled="textBusy" @click="textOpen = false">
                  取消
                </button>
              </div>
            </div>

            <div v-else key="list-view" class="list-view">
              <div v-if="items.length === 0" class="empty">
                <div class="empty-title">「{{ pod?.name ?? "匣" }}」是空的</div>
                <div class="empty-hint">把文件或图片拖到屏幕边缘的这个匣上<br />将按当前匣的动作设置暂存</div>
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

    <footer v-if="mode === 'list' && !textOpen" ref="footEl" class="panel-foot">
      <template v-if="selectedCount > 0">
        <span class="sel-count">已选 {{ selectedCount }} 项</span>
        <div class="foot-actions">
          <button type="button" class="foot-btn" :disabled="anyBusy" @click="exportSelected('copy')">
            复制到…
          </button>
          <button type="button" class="foot-btn" :disabled="anyBusy" @click="exportSelected('move')">
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
          <button type="button" class="foot-btn ghost" :disabled="anyBusy" @click="selectAll">全选</button>
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
  box-shadow: var(--shadow-panel), inset 0 1px 0 var(--glass-inner);
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
  from { opacity: 0; transform: scale(0.985); }
}
/* 自动隐藏：先淡出（后端延迟 220ms 才隐藏原生窗口），之后 forwards 保持
   透明，下次显示第一帧不闪现完整内容 */
.panel-root.panel-fade-out {
  animation: panel-fade-out 220ms ease both;
}
@keyframes panel-fade-out {
  to { opacity: 0; }
}

.panel-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 12px 6px;
  flex-shrink: 0;
  /* 背景与面板主体共用一整块半透明表面，不再单独分割出标题栏底色 */
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
  transition: background 120ms ease, color 120ms ease;
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
  transition: opacity 220ms ease, transform 280ms var(--ease-out);
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
  transition: transform 100ms ease, background 120ms ease;
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
  transition: background 120ms ease, color 120ms ease, border-color 120ms ease;
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
  transition: opacity 180ms ease, transform 240ms var(--ease-out);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(6px);
}

/* 内嵌降级菜单：覆盖面板窗口的可视区域，位置由 inlineMenuStyle 收敛 */
.inline-menu-layer {
  position: fixed;
  inset: 0;
  z-index: 60;
}
.inline-menu-pos {
  position: absolute;
}
</style>
