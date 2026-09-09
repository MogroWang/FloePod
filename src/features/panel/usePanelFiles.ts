import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { onBeforeUnmount, ref, type Ref } from "vue";
import type { StagedItem } from "@/domain/types";
import { confirmPreview } from "./confirmation";

export function usePanelFiles(
  context: PanelContext,
  listActionBusy: Ref<boolean>,
  selectedCount: Ref<number>,
  currentSelectedIds: () => number[],
  resetAnchor: () => void,
) {
  const { staging, showToast } = context;
  let confirmClearTimer: number | undefined;
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
      await ipc.revealStagedItems([item.id]);
    } catch (err) {
      console.error("reveal item failed", err);
      showToast("无法打开所在位置");
    }
  }

  async function removeItem(item: StagedItem) {
    await removeIds([item.id]);
  }

  async function removeSelected() {
    if (!selectedCount.value || context.isBusy()) return;
    await removeIds(currentSelectedIds());
  }

  async function removeIds(ids: number[]) {
    if (!ids.length || context.isBusy()) return;
    listActionBusy.value = true;
    try {
      const preview = await ipc.previewRemoveItems(ids, true);
      if (!(await confirmPreview(preview))) return;
      await staging.removeItems(ids, true);
      resetAnchor();
      showToast(`已移出 ${ids.length} 项，24 小时内可恢复`);
    } catch (err) {
      console.error("remove selected failed", err);
      showToast("移出失败，请重试");
    } finally {
      listActionBusy.value = false;
    }
  }

  const confirmClear = ref(false);
  async function clearAll() {
    if (context.isBusy()) return;
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
      resetAnchor();
      showToast("已清空（文件进回收站）");
    } catch (err) {
      console.error("clear pod failed", err);
      showToast("清空失败，请重试");
    } finally {
      listActionBusy.value = false;
    }
  }

  onBeforeUnmount(() => window.clearTimeout(confirmClearTimer));

  return { openItem, revealItem, removeItem, removeSelected, removeIds, confirmClear, clearAll };
}
