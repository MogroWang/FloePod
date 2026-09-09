import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { open } from "@tauri-apps/plugin-dialog";
import { ref, type Ref, type ComputedRef } from "vue";
import { exportVerb } from "@/domain/exportPresentation";
import type { DropAction, Pod, PanelMode } from "@/domain/types";

export function usePanelInput(
  context: PanelContext,
  pod: ComputedRef<Pod | undefined>,
  listActionBusy: Ref<boolean>,
  mode: Ref<PanelMode>,
  pendingPaths: Ref<string[]>,
) {
  const { showToast, refreshAfterMutation } = context;
  const askBusy = ref(false);
  async function stageAccessiblePaths(paths: string[]) {
    if (!paths.length || !pod.value) return;
    try {
      const configured = pod.value.dropAction;
      // Alt+数字是非拖拽投递入口（1.5.0 起始终注册）：用户主动触发即视为投递意图，
      // 「询问」模式下默认复制落地。
      const action = configured === "ask" ? "copy" : configured;
      if (!action) {
        await ipc.holdPendingDrop(context.podId(), paths);
        return;
      }
      const result = await ipc.stagePaths(context.podId(), paths, action);
      await refreshAfterMutation("暂存");
      showToast(
        result.warnings.length
          ? `已暂存，另有 ${result.warnings.length} 条提醒`
          : `已${action === "move" ? "移动" : action === "shortcut" ? "创建快捷方式" : "复制"} ${result.items.length} 项`,
      );
    } catch (error) {
      console.error("accessible stage failed", error);
      showToast(`暂存失败：${String(error)}`);
    }
  }

  async function pickPaths(directory: boolean) {
    if (context.isBusy()) return;
    listActionBusy.value = true;
    try {
      if (!ipc.inTauri) {
        await stageAccessiblePaths([directory ? "D:\\示例文件夹" : "D:\\示例文件.txt"]);
        return;
      }
      await ipc.setDraggingOut(context.podId(), true);
      const selected = await open({
        directory,
        multiple: !directory,
        title: directory ? "选择要暂存的文件夹" : "选择要暂存的文件",
      });
      const paths = Array.isArray(selected)
        ? selected
        : typeof selected === "string"
          ? [selected]
          : [];
      await stageAccessiblePaths(paths);
    } catch (error) {
      console.error("file picker failed", error);
      showToast("无法打开文件选择器，请重试");
    } finally {
      await ipc
        .setDraggingOut(context.podId(), false)
        .catch((error) => console.error("picker presence restore failed", error));
      listActionBusy.value = false;
    }
  }

  async function pasteClipboardFiles() {
    if (context.isBusy()) return;
    listActionBusy.value = true;
    try {
      const paths = await ipc.readClipboardFiles();
      if (!paths.length) {
        showToast("剪贴板里没有文件；请先在资源管理器中复制文件");
        return;
      }
      await stageAccessiblePaths(paths);
    } catch (error) {
      showToast(`无法读取剪贴板文件：${String(error)}`);
    } finally {
      listActionBusy.value = false;
    }
  }

  async function chooseAction(action: DropAction, remember: boolean) {
    if (context.isBusy()) return;
    const paths = [...pendingPaths.value];
    if (!paths.length || action === "ask") return;
    askBusy.value = true;
    try {
      const result = await ipc.stagePaths(context.podId(), paths, action);
      const verb = action === "shortcut" ? "快捷方式" : exportVerb(action);
      pendingPaths.value = [];
      mode.value = "list";
      const modeSynced = await ipc.setPanelMode(context.podId(), "list").then(
        () => true,
        (err) => {
          console.error("pending drop completion mode sync failed", err);
          return false;
        },
      );
      const refreshed = await refreshAfterMutation("暂存");
      if (!modeSynced) showToast("文件已暂存，但浮动面板状态同步失败");
      else if (result.warnings.length) {
        const warning = result.warnings[0];
        showToast(`已暂存，但 ${warning.name} 的源清理需检查：${warning.error}`);
      } else if (refreshed) showToast(`已暂存 ${paths.length} 项（${verb}）`);
      if (remember) {
        try {
          await context.settingsStore.updatePod(context.podId(), { dropAction: action });
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
    await ipc.setPanelMode(context.podId(), "list").catch((err) => {
      console.error("cancel pending drop failed", err);
      showToast("取消失败，请重试");
    });
  }

  return { askBusy, pickPaths, pasteClipboardFiles, chooseAction, cancelAsk };
}
