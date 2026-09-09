import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { computed, ref } from "vue";
import { buildItemMenu, type MenuItemSpec } from "@/domain/menu";
import type { SelectionMode } from "@/domain/selection";
import type { StagedItem } from "@/domain/types";

export function usePanelMenu(
  context: PanelContext,
  onSelect: (id: number, mode: SelectionMode) => void,
  removeIds: (ids: number[]) => Promise<void>,
) {
  const { staging, showToast } = context;
  const menuOpen = ref(false);
  const inlineMenu = ref<{ items: MenuItemSpec[]; x: number; y: number } | null>(null);

  function onItemContextMenu(item: StagedItem, at: { x: number; y: number }) {
    if (context.isBusy()) return;
    // 右键未选中的条目：先把选择收敛为该条目，与资源管理器一致。
    if (!staging.selectedIds.has(item.id)) onSelect(item.id, "set");
    const specs = buildItemMenu(staging.selectedItems);
    if (!specs.length) return;
    // 菜单已打开时右键其他条目：不拦截，直接开新菜单。后端 open 复用同一
    // 菜单窗口（seq 递增），旧菜单随之消失，并给被取代的旧归属匣补发关闭事件。
    menuOpen.value = true;
    // 菜单窗口会抢走指针：复用拖出保活语义避免浮动面板被看门狗收起，
    // 菜单关闭后由 CONTEXT_MENU_CLOSED 事件恢复。
    void ipc
      .setDraggingOut(context.podId(), true)
      .catch((err) => console.error("menu keep-alive failed", err));
    if (ipc.inTauri) {
      ipc.openContextMenu(context.podId(), specs).catch((err) => {
        // 菜单窗口未就绪等异常：降级为浮动面板内渲染，保证右键永远有反馈。
        console.error("menu window unavailable, using inline fallback", err);
        inlineMenu.value = { items: specs, x: at.x, y: at.y };
      });
    } else {
      inlineMenu.value = { items: specs, x: at.x, y: at.y };
    }
  }

  async function runMenuAction(spec: MenuItemSpec) {
    if (context.isBusy()) return;
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
          await removeIds(ids);
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
      .setDraggingOut(context.podId(), false)
      .catch((err) => console.error("menu presence restore failed", err));
  }

  function executeInlineMenu(spec: MenuItemSpec) {
    closeInlineMenu();
    void runMenuAction(spec);
  }

  /** 菜单显示期间的左键按下：收起菜单。菜单窗口的 blur 链路在点击不会
   *  激活的表面时可能不触发，这里显式收起兜底；右键（button=2）交给
   *  onItemContextMenu 的重入逻辑先关旧菜单再开新菜单，避免闪烁。 */
  function onGlobalPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    if (inlineMenu.value) {
      closeInlineMenu();
      return;
    }
    if (menuOpen.value && ipc.inTauri) {
      void ipc.dismissContextMenu().catch((err) => console.error("menu dismiss failed", err));
    }
  }

  return {
    menuOpen,
    inlineMenu,
    inlineMenuStyle,
    onItemContextMenu,
    runMenuAction,
    closeInlineMenu,
    executeInlineMenu,
    onGlobalPointerDown,
  };
}
