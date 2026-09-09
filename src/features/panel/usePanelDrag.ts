import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { ref } from "vue";
import { makeDragIcon } from "./dragIcon";
import { dragOut } from "@/domain/dragOut";

export function usePanelDrag(context: PanelContext, clearSelection: () => void) {
  const { staging, showToast, refreshAfterMutation } = context;
  const dragMode = ref<"copy" | "move">("copy");
  const dragBusy = ref(false);
  async function onDragOut(paths: string[]) {
    if (!paths.length || context.isBusy()) return;
    const first = staging.items.find((i) => i.stagingPath === paths[0]);
    const icon = makeDragIcon(paths, first?.ext ?? null);
    const requestedMode = dragMode.value;
    dragBusy.value = true;
    try {
      const result = await dragOut(requestedMode, {
        setActive: (active) => ipc.setDraggingOut(context.podId(), active),
        prepare: () => ipc.prepareDragCut(context.podId(), paths),
        drag: (mode) => ipc.startDragOut(paths, icon, mode),
        finalize: (token) => ipc.finalizeDragCut(token),
        cancel: (token) => ipc.cancelDragCut(token),
        cleanupFailed: (error) => console.error("drag cleanup failed", error),
      });
      if (result === "source-cleanup-failed") {
        await staging
          .refresh(context.podId())
          .catch((error) => console.error("post-drag refresh failed", error));
        showToast("目标已接收文件，但剪切源清理失败");
      } else if (result === "moved") {
        clearSelection();
        if (await refreshAfterMutation("剪切移出")) showToast(`已剪切移出 ${paths.length} 项`);
      }
    } catch (error) {
      console.error("drag out failed", error);
      showToast("拖出失败，请重试");
    } finally {
      dragBusy.value = false;
    }
  }
  return { dragMode, dragBusy, onDragOut };
}
