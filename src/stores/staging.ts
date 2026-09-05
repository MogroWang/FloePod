import { defineStore } from "pinia";
import type { ExportMode, ExportResult, StagedItem } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { Events, listenCurrent } from "@/ipc/events";

/** 暂存数据：条目 + 选中态（按当前匣过滤） */
export const useStagingStore = defineStore("staging", {
  state: () => ({
    items: [] as StagedItem[],
    activePodId: 0,
    selectedIds: new Set<number>(),
    refreshSeq: 0,
  }),

  getters: {
    activeItems(state): StagedItem[] {
      return state.items
        .filter((it) => it.podId === state.activePodId)
        .sort((a, b) => b.createdAt - a.createdAt);
    },
    selectedItems(state): StagedItem[] {
      return state.items.filter(
        (it) => it.podId === state.activePodId && state.selectedIds.has(it.id),
      );
    },
  },

  actions: {
    setActivePod(id: number) {
      this.activePodId = id;
      this.selectedIds.clear();
    },

    async refresh(podId?: number) {
      const pid = podId ?? this.activePodId;
      if (!pid) return;
      const request = ++this.refreshSeq;
      try {
        const items = await ipc.listPodItems(pid);
        // 刷新请求可能重叠；旧响应不能覆盖新快照，同时移除已经不存在的选中项。
        if (request !== this.refreshSeq || pid !== this.activePodId) return;
        this.items = items;
        const existing = new Set(items.map((item) => item.id));
        this.selectedIds = new Set([...this.selectedIds].filter((id) => existing.has(id)));
      } catch (err) {
        // 当前读取已被新请求或匣切换取代，不再向上抛出它的错误。
        if (request !== this.refreshSeq || pid !== this.activePodId) return;
        throw err;
      }
    },

    selectAll() {
      this.selectedIds = new Set(this.activeItems.map((i) => i.id));
    },

    clearSelection() {
      this.selectedIds.clear();
    },

    /** 文字暂存的公共序列（拖入文本 / 剪贴板收集 / 浮动面板收藏共用）。 */
    async stageTextAndRefresh(podId: number, content: string, title?: string) {
      await ipc.stageText(podId, content, title);
      await this.refresh(podId).catch((err) => {
        console.error("post-text-stage refresh failed", err);
      });
    },

    async removeItems(ids: number[], deleteFiles: boolean) {
      await ipc.removeItems(ids, deleteFiles);
      const removed = new Set(ids);
      this.selectedIds = new Set([...this.selectedIds].filter((id) => !removed.has(id)));
      this.items = this.items.filter((item) => !removed.has(item.id));
      // 删除已经成功，后续刷新失败不能诱导调用方重复删除；事件会继续触发同步。
      await this.refresh().catch((err) => console.error("post-remove refresh failed", err));
    },

    async clearActivePod(deleteFiles: boolean) {
      const ids = this.activeItems.map((i) => i.id);
      if (ids.length) await ipc.removeItems(ids, deleteFiles);
      this.selectedIds.clear();
      const removed = new Set(ids);
      this.items = this.items.filter((item) => !removed.has(item.id));
      await this.refresh().catch((err) => console.error("post-clear refresh failed", err));
    },

    async exportItems(ids: number[], destDir: string, mode: ExportMode): Promise<ExportResult> {
      return ipc.exportItems(ids, destDir, mode, "ask");
    },

    setSelection(ids: number[]) {
      const allowed = new Set(this.activeItems.map((item) => item.id));
      this.selectedIds = new Set(ids.filter((id) => allowed.has(id)));
    },

    async listenChanges(podId: number) {
      return listenCurrent<{ podId: number }>(Events.ItemsChanged, (p) => {
        if (!p.podId || p.podId === podId) {
          void this.refresh(podId).catch((err) => console.error("staging refresh failed", err));
        }
      });
    },
  },
});
