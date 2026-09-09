import { computed, watch } from "vue";
import type { PanelContext } from "./context";
import { updateSelection, type SelectionMode } from "@/domain/selection";
import type { StagedItem } from "@/domain/types";

export function usePanelSelection(context: PanelContext) {
  const { staging } = context;
  let anchorId: number | null = null;
  const items = computed(() => staging.activeItems);
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

  function onSelect(id: number, mode: SelectionMode) {
    // 导出、拖出和删除会按启动时的选择处理；执行期间锁定选择，避免完成时覆盖新选择。
    if (context.isBusy()) return;
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

  return {
    items,
    selectedItems,
    selectedCount,
    currentSelectedIds,
    clearSelection,
    selectAll,
    onSelect,
    selectedOrSingle,
    resetAnchor: () => {
      anchorId = null;
    },
  };
}
