import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { ref, type Ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { BROWSER_PREVIEW_EXPORT_ROOT } from "@/lib/env";
import { exportVerb, presentExport } from "@/domain/exportPresentation";
import type { ConflictStrategy, ExportMode, ExportResult, PanelMode } from "@/domain/types";
import { confirmPreview } from "./confirmation";

export function usePanelExport(
  context: PanelContext,
  mode: Ref<PanelMode>,
  currentSelectedIds: () => number[],
  resetAnchor: () => void,
) {
  const { staging, showToast, refreshAfterMutation } = context;
  const exportBusy = ref(false);
  const conflict = ref<{ names: string[]; ids: number[]; dest: string; mode: ExportMode } | null>(
    null,
  );
  async function pickDest(): Promise<string | null> {
    if (!ipc.inTauri) return BROWSER_PREVIEW_EXPORT_ROOT;
    const dir = await open({ directory: true, multiple: false, title: "选择目标文件夹" });
    return typeof dir === "string" ? dir : null;
  }

  async function applyExportResult(result: ExportResult, exportMode: ExportMode) {
    const verb = exportVerb(exportMode);
    const presentation = presentExport(result, exportMode);
    if (presentation.selection !== null) {
      staging.setSelection(presentation.selection);
      resetAnchor();
    }
    const refreshed = await refreshAfterMutation(verb);
    let message = presentation.message;
    if (!refreshed) message += "；列表刷新失败";
    showToast(message);
  }

  async function exportSelected(exportMode: ExportMode) {
    const ids = currentSelectedIds();
    if (!ids.length || context.isBusy()) return;
    exportBusy.value = true;
    try {
      // 原生目录选择器会让指针离开 WebView；操作期间保持浮动面板可见。
      await ipc.setDraggingOut(context.podId(), true);
      const dest = await pickDest();
      if (!dest) return;
      const preview = await ipc.previewExportItems(ids, dest, exportMode);
      if (!(await confirmPreview(preview))) return;
      const result = await staging.exportItems(ids, dest, exportMode);
      if (result.conflicts.length > 0) {
        conflict.value = { names: result.conflicts, ids, dest, mode: exportMode };
        mode.value = "conflict";
        await ipc.setPanelMode(context.podId(), "conflict").catch((err) => {
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
        await ipc.setPanelMode(context.podId(), "list").catch(() => {});
      }
      showToast("导出失败，请重试");
    } finally {
      await ipc.setDraggingOut(context.podId(), false).catch((err) => {
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
      const modeSynced = await ipc.setPanelMode(context.podId(), "list").then(
        () => true,
        (err) => {
          console.error("conflict completion mode sync failed", err);
          return false;
        },
      );
      await applyExportResult(result, ctx.mode);
      if (!modeSynced) showToast("导出已处理，但浮动面板状态同步失败");
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
    await ipc.setPanelMode(context.podId(), "list").catch((err) => {
      console.error("cancel conflict failed", err);
    });
  }

  return { conflict, exportBusy, exportSelected, resolveConflict, cancelConflict };
}
