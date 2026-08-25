import { Channel, invoke as invokeTauri } from "@tauri-apps/api/core";

import type {
  Bootstrap,
  ConflictStrategy,
  DragCutToken,
  DropAction,
  ExportMode,
  ExportResult,
  Hotkeys,
  ModifierState,
  MonitorInfo,
  PanelMode,
  PanelState,
  Pod,
  Settings,
  StagePathsResult,
  StagedItem,
  ThumbnailPayload,
} from "@/domain/types";
import { Commands, type CommandName } from "./commands";
import { mockInvoke } from "./mock";

const inTauri = "__TAURI_INTERNALS__" in window;

function invoke<T>(command: CommandName, args?: Record<string, unknown>): Promise<T> {
  return inTauri ? invokeTauri<T>(command, args) : mockInvoke<T>(command, args);
}

export const ipc = {
  inTauri,

  getBootstrap: (): Promise<Bootstrap> => invoke(Commands.GetBootstrap),
  getPod: (podId: number): Promise<Pod | null> => invoke(Commands.GetPod, { podId }),
  getMonitors: (): Promise<MonitorInfo[]> => invoke(Commands.GetMonitors),
  getHotkeyDefaults: (): Promise<Hotkeys> => invoke(Commands.GetHotkeyDefaults),
  getModifierState: (): Promise<ModifierState> => invoke(Commands.GetModifierState),

  createPod: (config: Partial<Pod>, reuseExisting = false): Promise<Pod> =>
    invoke(Commands.CreatePod, { config, reuseExisting }),
  updatePod: (podId: number, patch: Partial<Pod>): Promise<Pod | null> =>
    invoke(Commands.UpdatePod, { podId, patch }),
  deletePod: (podId: number, recycleFiles: boolean): Promise<void> =>
    invoke(Commands.DeletePod, { podId, recycleFiles }),
  saveSettings: (patch: Partial<Settings>): Promise<Settings> =>
    invoke(Commands.SaveSettings, { patch }),

  holdPendingDrop: (podId: number, paths: string[]): Promise<void> =>
    invoke(Commands.HoldPendingDrop, { podId, paths }),
  stagePaths: (podId: number, paths: string[], action: DropAction): Promise<StagePathsResult> =>
    invoke(Commands.StagePaths, { podId, paths, action }),
  stageText: (podId: number, content: string, title?: string): Promise<StagedItem> =>
    invoke(Commands.StageText, { podId, content, title: title ?? null }),
  listPodItems: (podId: number): Promise<StagedItem[]> =>
    invoke(Commands.ListPodItems, { podId }),
  removeItems: (ids: number[], deleteFiles: boolean): Promise<void> =>
    invoke(Commands.RemoveItems, { ids, deleteFiles }),

  exportItems: (
    ids: number[],
    destDir: string,
    mode: ExportMode,
    onConflict: ConflictStrategy,
  ): Promise<ExportResult> =>
    invoke(Commands.ExportItems, { ids, destDir, mode, onConflict }),
  readThumbnail: (path: string): Promise<ThumbnailPayload | null> =>
    invoke(Commands.ReadThumbnail, { path }),

  showPanel: (podId: number): Promise<void> => invoke(Commands.ShowPanel, { podId }),
  togglePanel: (podId: number): Promise<void> => invoke(Commands.TogglePanel, { podId }),
  hidePanel: (podId: number): Promise<void> => invoke(Commands.HidePanel, { podId }),
  setPanelMode: (podId: number, mode: PanelMode): Promise<void> =>
    invoke(Commands.SetPanelMode, { podId, mode }),
  getPanelState: (podId: number): Promise<PanelState> =>
    invoke(Commands.GetPanelState, { podId }),
  reportPresence: (podId: number, window: string, inside: boolean): Promise<void> =>
    invoke(Commands.ReportPresence, { podId, window, inside }),
  setPanelPinned: (podId: number, pinned: boolean): Promise<void> =>
    invoke(Commands.SetPanelPinned, { podId, pinned }),
  setDraggingOut: (podId: number, dragging: boolean): Promise<void> =>
    invoke(Commands.SetDraggingOut, { podId, dragging }),
  setPodAccept: (podId: number, accepting: boolean): Promise<void> =>
    invoke(Commands.SetPodAccept, { podId, accepting }),
  setPanelSize: (podId: number, width: number, height: number): Promise<void> =>
    invoke(Commands.SetPanelSize, { podId, width, height }),
  movePodBar: (podId: number, offset: number): Promise<void> =>
    invoke(Commands.MovePodBar, { podId, offset }),
  toggleAllBars: (): Promise<void> => invoke(Commands.ToggleAllBars),
  openSettings: (): Promise<void> => invoke(Commands.OpenSettings),
  logFrontend: (msg: string): Promise<void> => invoke(Commands.LogFrontend, { msg }),
  appLog: (msg: string): Promise<void> => invoke(Commands.AppLog, { msg }),
  quitApp: (): Promise<void> => invoke(Commands.QuitApp),

  startDragOut: async (
    paths: string[],
    iconDataUrl: string,
    mode: "copy" | "move",
  ): Promise<boolean> => {
    if (!inTauri) return false;
    let resolveResult!: (dropped: boolean) => void;
    const result = new Promise<boolean>((resolve) => {
      resolveResult = resolve;
    });
    const channel = new Channel<{ result: { type?: string } | string }>();
    channel.onmessage = (message) => {
      const value = message?.result;
      const kind = typeof value === "string" ? value : (value?.type ?? "");
      resolveResult(kind === "Dropped" || kind === "dropped");
    };
    await invokeTauri("plugin:drag|start_drag", {
      item: paths,
      image: iconDataUrl,
      options: { mode },
      onEvent: channel,
    });
    return result;
  },

  prepareDragCut: (podId: number, paths: string[]): Promise<DragCutToken> =>
    invoke(Commands.PrepareDragCut, { podId, paths }),
  finalizeDragCut: (token: DragCutToken): Promise<void> =>
    invoke(Commands.FinalizeDragCut, { token }),
  cancelDragCut: (token: DragCutToken): Promise<void> =>
    invoke(Commands.CancelDragCut, { token }),
};

export type Ipc = typeof ipc;
