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
    lastError: "" as string,
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
        // ItemsChanged events can overlap. Never let an older response replace a
        // newer snapshot, and discard selections which no longer exist on disk.
        if (request !== this.refreshSeq || pid !== this.activePodId) return;
        this.items = items;
        const existing = new Set(items.map((item) => item.id));
        this.selectedIds = new Set([...this.selectedIds].filter((id) => existing.has(id)));
        this.lastError = "";
      } catch (err) {
        // A newer request (or a pod switch) has already superseded this read.
        // Its failure must not surface as the result of the current snapshot.
        if (request !== this.refreshSeq || pid !== this.activePodId) return;
        this.lastError = err instanceof Error ? err.message : String(err);
        throw err;
      }
    },

    toggleSelected(id: number, additive: boolean) {
      if (!additive) this.selectedIds.clear();
      if (this.selectedIds.has(id)) this.selectedIds.delete(id);
      else this.selectedIds.add(id);
    },

    selectAll() {
      this.selectedIds = new Set(this.activeItems.map((i) => i.id));
    },

    clearSelection() {
      this.selectedIds.clear();
    },

    async removeItems(ids: number[], deleteFiles: boolean) {
      await ipc.removeItems(ids, deleteFiles);
      const removed = new Set(ids);
      this.selectedIds = new Set([...this.selectedIds].filter((id) => !removed.has(id)));
      this.items = this.items.filter((item) => !removed.has(item.id));
      // The destructive operation already succeeded. A failed follow-up read
      // must not make callers retry deletion; ItemsChanged can reconcile later.
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
