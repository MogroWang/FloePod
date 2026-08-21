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
} from "@/types";

/**
 * Tauri IPC 封装。
 * 在浏览器中（vite dev）自动切换到 mock 实现，便于无 Tauri 环境的 UI 开发。
 */

const inTauri = "__TAURI_INTERNALS__" in window;

/** 浏览器开发模式的轻量 mock（无 Tauri 时保证 UI 可预览） */
let mockItems: import("@/types").StagedItem[] = [];
const MOCK_HOTKEY_DEFAULTS: Hotkeys = {
  toggleBar: "Alt+Shift+F",
  collectClipboard: "Alt+Shift+S",
  openPanel: "Alt+Shift+P",
};
let mockSettings: import("@/types").Settings = {
  theme: "system",
  firstRunDone: false,
  autostart: false,
  hotkeys: { ...MOCK_HOTKEY_DEFAULTS },
  pods: [
    {
      id: 1,
      name: "我的匣",
      edge: "left",
      monitor: "",
      offset: 0.5,
      stagingFolder: "D:\\浮匣暂存（浏览器预览）",
      opacity: 0.85,
      material: "acrylic",
      panelWidth: 380,
      hoverDelayMs: 120,
      dropAction: "ask",
      enabled: true,
    },
  ],
  version: "0.5.1-mock",
  dataDir: "浏览器预览",
};
const mockMonitors: MonitorInfo[] = [
  { name: "\\\\.\\DISPLAY1", label: "主显示器", primary: true },
];
const mockPanelStates = new Map<number, PanelState>();
let mockDragCutSequence = 0;
const mockDragCuts = new Map<DragCutToken, { podId: number; paths: string[] }>();

function mockPanelState(podId: number): PanelState {
  let state = mockPanelStates.get(podId);
  if (!state) {
    state = { mode: "list", paths: [], pinned: false, visible: false, draggingOut: false };
    mockPanelStates.set(podId, state);
  }
  return state;
}

if (!inTauri) {
  mockItems = [
    {
      id: 1, podId: 1, kind: "file", stagingPath: "D:\\staging\\blueprint.webp",
      originalPath: "C:\\src\\blueprint.webp", name: "蓝图参考.webp", ext: "webp", size: 245760, createdAt: Date.now() - 6e5,
    },
    {
      id: 2, podId: 1, kind: "file", stagingPath: "D:\\staging\\需求说明.pdf",
      originalPath: null, name: "需求说明.pdf", ext: "pdf", size: 512000, createdAt: Date.now() - 18e5,
    },
    {
      id: 3, podId: 1, kind: "text", stagingPath: "D:\\staging\\会议要点.txt",
      originalPath: null, name: "会议要点.txt", ext: "txt", size: 2048, createdAt: Date.now() - 4e6,
    },
    {
      id: 4, podId: 1, kind: "folder", stagingPath: "D:\\staging\\素材包",
      originalPath: "C:\\src\\素材包", name: "素材包", ext: null, size: 0, createdAt: Date.now() - 2e6,
    },
    {
      id: 5, podId: 1, kind: "shortcut", stagingPath: "D:\\staging\\原型.fig - 快捷方式.lnk",
      originalPath: "C:\\src\\原型.fig", name: "原型.fig - 快捷方式.lnk", ext: "lnk", size: 0, createdAt: Date.now() - 3e5,
    },
  ];
}

async function invoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inTauri) return mockInvoke<T>(cmd, args);
  const { invoke } = await import("@tauri-apps/api/core");
  return invoke<T>(cmd, args);
}

async function mockInvoke<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  const seed = () => (mockItems = [...mockItems]);
  const ret = (v: unknown) => v as T;
  switch (cmd) {
    case "get_bootstrap":
      return ret({ settings: mockSettings, monitors: mockMonitors, version: "0.5.1-mock" });
    case "get_pod":
      return ret(mockSettings.pods.find((p) => p.id === Number(args?.podId)) ?? null);
    case "get_monitors":
      return ret(mockMonitors);
    case "get_hotkey_defaults":
      return ret({ ...MOCK_HOTKEY_DEFAULTS });
    case "get_modifier_state":
      return ret({ ctrl: false, shift: false, alt: false });
    case "list_pod_items":
      return ret(mockItems.filter((i) => i.podId === Number(args?.podId)));
    case "get_panel_state": {
      const state = mockPanelState(Number(args?.podId));
      return ret({ ...state, paths: [...state.paths] });
    }
    case "create_pod": {
      if (args?.reuseExisting) {
        const folder = String((args?.config as Partial<Pod> | undefined)?.stagingFolder ?? "");
        const existing = mockSettings.pods.find((pod) => pod.stagingFolder === folder);
        if (existing) return ret(existing);
      }
      const pod: Pod = {
        id: mockSettings.pods.reduce((m, p) => Math.max(m, p.id), 0) + 1,
        name: "新匣",
        edge: "left",
        monitor: "",
        offset: 0.5,
        stagingFolder: "",
        opacity: 0.85,
        material: "acrylic",
        panelWidth: 380,
        hoverDelayMs: 120,
        dropAction: "ask",
        enabled: true,
        ...(args?.config as object),
      };
      mockSettings = { ...mockSettings, pods: [...mockSettings.pods, pod] };
      return ret(pod);
    }
    case "update_pod": {
      const id = Number(args?.podId);
      const patch = (args?.patch as object) ?? {};
      mockSettings = {
        ...mockSettings,
        pods: mockSettings.pods.map((p) => (p.id === id ? { ...p, ...patch } : p)),
      };
      return ret(mockSettings.pods.find((p) => p.id === id) ?? null);
    }
    case "delete_pod": {
      const id = Number(args?.podId);
      mockSettings = { ...mockSettings, pods: mockSettings.pods.filter((p) => p.id !== id) };
      mockItems = mockItems.filter((item) => item.podId !== id);
      mockPanelStates.delete(id);
      for (const [token, cut] of mockDragCuts) {
        if (cut.podId === id) mockDragCuts.delete(token);
      }
      return ret(undefined);
    }
    case "save_settings":
      mockSettings = { ...mockSettings, ...((args?.patch as object) ?? {}) };
      return ret(mockSettings);
    case "stage_text": {
      const content = String(args?.content ?? "");
      const requestedTitle = String(args?.title ?? "").trim().replace(/\.txt$/i, "");
      const baseName = requestedTitle || `文字 ${mockItems.length + 1}`;
      const item: import("@/types").StagedItem = {
        id: Date.now(), podId: Number(args?.podId) || 1, kind: "text",
        stagingPath: `D:\\staging\\${baseName}.txt`,
        originalPath: null, name: `${baseName}.txt`, ext: "txt",
        size: content.length, createdAt: Date.now(),
      };
      seed().push(item);
      return ret(item);
    }
    case "stage_paths": {
      const podId = Number(args?.podId) || 1;
      const action = String(args?.action ?? "copy") as Exclude<DropAction, "ask">;
      const paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      const pod = mockSettings.pods.find((candidate) => candidate.id === podId);
      let nextId = mockItems.reduce((max, item) => Math.max(max, item.id), 0) + 1;
      const created = paths.map((path) => {
        const sourceName = path.split(/[\\/]/).pop() || `项目 ${nextId}`;
        const shortcut = action === "shortcut";
        const name = shortcut ? `${sourceName} - 快捷方式.lnk` : sourceName;
        const dot = name.lastIndexOf(".");
        const item: StagedItem = {
          id: nextId++,
          podId,
          kind: shortcut ? "shortcut" : "file",
          stagingPath: `${pod?.stagingFolder || "D:\\staging"}\\${name}`,
          originalPath: path,
          name,
          ext: dot > 0 ? name.slice(dot + 1).toLowerCase() : null,
          size: 0,
          createdAt: Date.now(),
        };
        return item;
      });
      mockItems = [...mockItems, ...created];
      return ret({ items: created, warnings: [] } satisfies StagePathsResult);
    }
    case "hold_pending_drop": {
      const state = mockPanelState(Number(args?.podId));
      state.mode = "ask";
      state.paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      state.visible = true;
      return ret(undefined);
    }
    case "remove_items": {
      const ids = new Set(Array.isArray(args?.ids) ? args.ids.map(Number) : []);
      mockItems = mockItems.filter((item) => !ids.has(item.id));
      return ret(undefined);
    }
    case "export_items": {
      const ids = Array.isArray(args?.ids) ? args.ids.map(Number) : [];
      if (args?.mode === "move") {
        const moved = new Set(ids);
        mockItems = mockItems.filter((item) => !moved.has(item.id));
      }
      return ret({
        conflicts: [],
        completedIds: ids,
        skippedIds: [],
        staleIds: [],
        failed: [],
        warnings: [],
      } satisfies ExportResult);
    }
    case "read_thumbnail":
      return ret(null);
    case "show_panel": {
      mockPanelState(Number(args?.podId)).visible = true;
      return ret(undefined);
    }
    case "toggle_panel": {
      const state = mockPanelState(Number(args?.podId));
      if (!state.visible) {
        state.visible = true;
        state.pinned = true;
      } else if (state.pinned) {
        Object.assign(state, { mode: "list", paths: [], pinned: false, visible: false, draggingOut: false });
      } else {
        state.pinned = true;
      }
      return ret(undefined);
    }
    case "hide_panel": {
      const state = mockPanelState(Number(args?.podId));
      Object.assign(state, { mode: "list", paths: [], pinned: false, visible: false, draggingOut: false });
      return ret(undefined);
    }
    case "set_panel_mode": {
      const state = mockPanelState(Number(args?.podId));
      state.mode = String(args?.mode ?? "list") as PanelMode;
      if (state.mode === "list") state.paths = [];
      return ret(undefined);
    }
    case "set_panel_pinned": {
      const state = mockPanelState(Number(args?.podId));
      state.pinned = Boolean(args?.pinned);
      if (state.pinned) state.visible = true;
      return ret(undefined);
    }
    case "set_dragging_out":
      mockPanelState(Number(args?.podId)).draggingOut = Boolean(args?.dragging);
      return ret(undefined);
    case "prepare_drag_cut": {
      const podId = Number(args?.podId);
      const paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      if (!paths.length || new Set(paths).size !== paths.length) {
        throw new Error("剪切列表为空或包含重复路径");
      }
      if (paths.some((path) => !mockItems.some((item) => item.podId === podId && item.stagingPath === path))) {
        throw new Error("剪切路径不属于当前匣");
      }
      const token: DragCutToken = `mock-cut-${++mockDragCutSequence}`;
      mockDragCuts.set(token, { podId, paths: [...paths] });
      return ret(token);
    }
    case "finalize_drag_cut": {
      const token = String(args?.token ?? "") as DragCutToken;
      const cut = mockDragCuts.get(token);
      mockDragCuts.delete(token);
      if (!cut) throw new Error("剪切令牌无效或已被使用");
      const paths = new Set(cut.paths);
      mockItems = mockItems.filter(
        (item) => item.podId !== cut.podId || !paths.has(item.stagingPath),
      );
      return ret(undefined);
    }
    case "cancel_drag_cut": {
      mockDragCuts.delete(String(args?.token ?? "") as DragCutToken);
      return ret(undefined);
    }
    default:
      // 窗口类命令在浏览器里静默成功即可
      return ret(undefined);
  }
}

export const ipc = {
  inTauri,

  getBootstrap: (): Promise<Bootstrap> => invoke("get_bootstrap"),
  getPod: (podId: number): Promise<Pod | null> => invoke("get_pod", { podId }),
  getMonitors: (): Promise<MonitorInfo[]> => invoke("get_monitors"),

  // ---- 匣 CRUD ----
  createPod: (config: Partial<Pod>, reuseExisting = false): Promise<Pod> =>
    invoke("create_pod", { config, reuseExisting }),
  updatePod: (podId: number, patch: Partial<Pod>): Promise<Pod | null> =>
    invoke("update_pod", { podId, patch }),
  deletePod: (podId: number, recycleFiles: boolean): Promise<void> =>
    invoke("delete_pod", { podId, recycleFiles }),

  // ---- 拖入 ----
  getModifierState: (): Promise<ModifierState> => invoke("get_modifier_state"),
  /** 拖入待询问：暂存路径并弹出该匣面板 ask 模式 */
  holdPendingDrop: (podId: number, paths: string[]): Promise<void> =>
    invoke("hold_pending_drop", { podId, paths }),
  /** 确认动作后执行暂存 */
  stagePaths: (podId: number, paths: string[], action: DropAction): Promise<StagePathsResult> =>
    invoke("stage_paths", { podId, paths, action }),
  stageText: (podId: number, content: string, title?: string): Promise<StagedItem> =>
    invoke("stage_text", { podId, content, title: title ?? null }),

  // ---- 列表 ----
  listPodItems: (podId: number): Promise<StagedItem[]> =>
    invoke("list_pod_items", { podId }),
  removeItems: (ids: number[], deleteFiles: boolean): Promise<void> =>
    invoke("remove_items", { ids, deleteFiles }),

  // ---- 导出 ----
  exportItems: (
    ids: number[],
    destDir: string,
    mode: ExportMode,
    onConflict: ConflictStrategy,
  ): Promise<ExportResult> => invoke("export_items", { ids, destDir, mode, onConflict }),

  // ---- 缩略图 ----
  readThumbnail: (path: string): Promise<ThumbnailPayload | null> =>
    invoke("read_thumbnail", { path }),

  // ---- 设置 ----
  saveSettings: (patch: Partial<Settings>): Promise<Settings> =>
    invoke("save_settings", { patch }),
  getHotkeyDefaults: (): Promise<Hotkeys> => invoke("get_hotkey_defaults"),

  // ---- 窗口（按匣） ----
  showPanel: (podId: number): Promise<void> => invoke("show_panel", { podId }),
  togglePanel: (podId: number): Promise<void> => invoke("toggle_panel", { podId }),
  hidePanel: (podId: number): Promise<void> => invoke("hide_panel", { podId }),
  setPanelMode: (podId: number, mode: PanelMode): Promise<void> =>
    invoke("set_panel_mode", { podId, mode }),
  getPanelState: (podId: number): Promise<PanelState> =>
    invoke("get_panel_state", { podId }),
  reportPresence: (podId: number, window: string, inside: boolean): Promise<void> =>
    invoke("report_presence", { podId, window, inside }),
  setPanelPinned: (podId: number, pinned: boolean): Promise<void> =>
    invoke("set_panel_pinned", { podId, pinned }),
  setDraggingOut: (podId: number, dragging: boolean): Promise<void> =>
    invoke("set_dragging_out", { podId, dragging }),
  setPodAccept: (podId: number, accepting: boolean): Promise<void> =>
    invoke("set_pod_accept", { podId, accepting }),
  toggleAllBars: (): Promise<void> => invoke("toggle_all_bars"),
  openSettings: (): Promise<void> => invoke("open_settings"),
  logFrontend: (msg: string): Promise<void> => invoke("log_frontend", { msg }),
  appLog: (msg: string): Promise<void> => invoke("app_log", { msg }),
  quitApp: (): Promise<void> => invoke("quit_app"),
  setPanelSize: (podId: number, width: number, height: number): Promise<void> =>
    invoke("set_panel_size", { podId, width, height }),

  // ---- 拖出（tauri-plugin-drag 底层命令） ----
  startDragOut: async (
    paths: string[],
    iconDataUrl: string,
    mode: "copy" | "move",
  ): Promise<boolean> => {
    if (!inTauri) return false;
    const { Channel, invoke: rawInvoke } = await import("@tauri-apps/api/core");
    let resolveResult!: (dropped: boolean) => void;
    const resultPromise = new Promise<boolean>((resolve) => {
      resolveResult = resolve;
    });
    const channel = new Channel<{ result: { type?: string } | string }>();
    channel.onmessage = (msg) => {
      const r = msg?.result;
      const kind = typeof r === "string" ? r : (r?.type ?? "");
      resolveResult(kind === "Dropped" || kind === "dropped");
    };
    await rawInvoke("plugin:drag|start_drag", {
      item: paths,
      image: iconDataUrl,
      options: { mode },
      onEvent: channel,
    });
    return resultPromise;
  },

  /** OLE Move 前捕获一次性文件身份快照。 */
  prepareDragCut: (podId: number, paths: string[]): Promise<DragCutToken> =>
    invoke("prepare_drag_cut", { podId, paths }),
  /** 目标接收后消费令牌并仅删除身份未变化的源。 */
  finalizeDragCut: (token: DragCutToken): Promise<void> =>
    invoke("finalize_drag_cut", { token }),
  /** 拖拽取消或异常时撤销尚未消费的令牌。 */
  cancelDragCut: (token: DragCutToken): Promise<void> =>
    invoke("cancel_drag_cut", { token }),
};

export type Ipc = typeof ipc;
