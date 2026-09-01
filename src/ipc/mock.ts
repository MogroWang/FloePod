import type {
  DragCutToken,
  DropAction,
  ExportResult,
  Hotkeys,
  MonitorInfo,
  PanelMode,
  PanelState,
  Pod,
  Settings,
  StagePathsResult,
  StagedItem,
} from "@/domain/types";
import { Commands, type CommandName } from "./commands";
import { BROWSER_PREVIEW_STAGING_ROOT } from "@/lib/env";

const HOTKEY_DEFAULTS: Hotkeys = {
  toggleBar: "Alt+Shift+F",
  collectClipboard: "Alt+Shift+S",
  openPanel: "Alt+Shift+P",
};

let settings: Settings = {
  theme: "system",
  firstRunDone: false,
  autostart: false,
  hotkeys: { ...HOTKEY_DEFAULTS },
  autoBlock: { enabled: false, apps: [] },
  pods: [
    {
      id: 1,
      name: "我的匣",
      edge: "left",
      monitor: "",
      offset: 0.5,
      stagingFolder: BROWSER_PREVIEW_STAGING_ROOT,
      opacity: 1,
      material: "acrylic",
      panelMaterial: "acrylic",
      panelOpacity: 1,
      panelWidth: 380,
      hoverDelayMs: 120,
      autoHide: true,
      autoHideDelayMs: 320,
      dropAction: "ask",
      enabled: true,
      barWidth: 44,
      cornerRadius: 22,
      borderColor: "",
      borderOpacity: 1,
    },
  ],
  version: "1.3.0-mock",
  dataDir: "浏览器预览",
};

const monitors: MonitorInfo[] = [
  {
    name: "\\\\.\\DISPLAY1",
    label: "显示器 1（主）",
    primary: true,
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
    scaleFactor: 1,
  },
];

let items: StagedItem[] = [
  {
    id: 1,
    podId: 1,
    kind: "file",
    stagingPath: "D:\\staging\\blueprint.webp",
    originalPath: "C:\\src\\blueprint.webp",
    name: "蓝图参考.webp",
    ext: "webp",
    size: 245760,
    createdAt: Date.now() - 6e5,
  },
  {
    id: 2,
    podId: 1,
    kind: "file",
    stagingPath: "D:\\staging\\需求说明.pdf",
    originalPath: null,
    name: "需求说明.pdf",
    ext: "pdf",
    size: 512000,
    createdAt: Date.now() - 18e5,
  },
  {
    id: 3,
    podId: 1,
    kind: "text",
    stagingPath: "D:\\staging\\会议要点.txt",
    originalPath: null,
    name: "会议要点.txt",
    ext: "txt",
    size: 2048,
    createdAt: Date.now() - 4e6,
  },
  {
    id: 4,
    podId: 1,
    kind: "folder",
    stagingPath: "D:\\staging\\素材包",
    originalPath: "C:\\src\\素材包",
    name: "素材包",
    ext: null,
    size: 0,
    createdAt: Date.now() - 2e6,
  },
  {
    id: 5,
    podId: 1,
    kind: "shortcut",
    stagingPath: "D:\\staging\\原型.fig - 快捷方式.lnk",
    originalPath: "C:\\src\\原型.fig",
    name: "原型.fig - 快捷方式.lnk",
    ext: "lnk",
    size: 0,
    createdAt: Date.now() - 3e5,
  },
];

const panelStates = new Map<number, PanelState>();
const cuts = new Map<DragCutToken, { podId: number; paths: string[] }>();
let cutSequence = 0;

function panelState(podId: number): PanelState {
  let state = panelStates.get(podId);
  if (!state) {
    state = { mode: "list", paths: [], pinned: false, visible: false, draggingOut: false };
    panelStates.set(podId, state);
  }
  return state;
}
export async function mockInvoke<T>(command: CommandName, args?: Record<string, unknown>): Promise<T> {
  const result = (value: unknown) => value as T;
  switch (command) {
    case Commands.GetBootstrap:
      return result({ settings, monitors, version: settings.version });
    case Commands.GetHotkeyDefaults:
      return result({ ...HOTKEY_DEFAULTS });
    case Commands.GetModifierState:
      return result({ ctrl: false, shift: false, alt: false });
    case Commands.ListPodItems:
      return result(items.filter((item) => item.podId === Number(args?.podId)));
    case Commands.GetPanelState: {
      const state = panelState(Number(args?.podId));
      return result({ ...state, paths: [...state.paths] });
    }
    case Commands.CreatePod: {
      if (args?.reuseExisting) {
        const folder = String((args.config as Partial<Pod> | undefined)?.stagingFolder ?? "");
        const existing = settings.pods.find((pod) => pod.stagingFolder === folder);
        if (existing) return result(existing);
      }
      const pod: Pod = {
        id: settings.pods.reduce((max, pod) => Math.max(max, pod.id), 0) + 1,
        name: "新匣",
        edge: "left",
        monitor: "",
        offset: 0.5,
        stagingFolder: "",
        opacity: 1,
        material: "acrylic",
        panelMaterial: "acrylic",
        panelOpacity: 1,
        panelWidth: 380,
        hoverDelayMs: 120,
        autoHide: true,
        autoHideDelayMs: 320,
        dropAction: "ask",
        enabled: true,
        barWidth: 44,
        cornerRadius: 22,
        borderColor: "",
        borderOpacity: 1,
        ...(args?.config as object),
      };
      settings = { ...settings, pods: [...settings.pods, pod] };
      return result(pod);
    }
    case Commands.UpdatePod: {
      const id = Number(args?.podId);
      const patch = (args?.patch as object) ?? {};
      settings = {
        ...settings,
        pods: settings.pods.map((pod) => (pod.id === id ? { ...pod, ...patch } : pod)),
      };
      return result(settings.pods.find((pod) => pod.id === id) ?? null);
    }
    case Commands.DeletePod: {
      const id = Number(args?.podId);
      settings = { ...settings, pods: settings.pods.filter((pod) => pod.id !== id) };
      items = items.filter((item) => item.podId !== id);
      panelStates.delete(id);
      for (const [token, cut] of cuts) if (cut.podId === id) cuts.delete(token);
      return result(undefined);
    }
    case Commands.SaveSettings:
      settings = { ...settings, ...((args?.patch as object) ?? {}) };
      return result(settings);
    case Commands.StageText: {
      const content = String(args?.content ?? "");
      const requested = String(args?.title ?? "").trim().replace(/\.txt$/i, "");
      const base = requested || `文字 ${items.length + 1}`;
      const item: StagedItem = {
        id: Date.now(),
        podId: Number(args?.podId) || 1,
        kind: "text",
        stagingPath: `D:\\staging\\${base}.txt`,
        originalPath: null,
        name: `${base}.txt`,
        ext: "txt",
        size: content.length,
        createdAt: Date.now(),
      };
      items = [...items, item];
      return result(item);
    }
    case Commands.StagePaths: {
      const podId = Number(args?.podId) || 1;
      const action = String(args?.action ?? "copy") as Exclude<DropAction, "ask">;
      const paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      const pod = settings.pods.find((candidate) => candidate.id === podId);
      let nextId = items.reduce((max, item) => Math.max(max, item.id), 0) + 1;
      const created = paths.map((path): StagedItem => {
        const sourceName = path.split(/[\\/]/).pop() || `项目 ${nextId}`;
        const shortcut = action === "shortcut";
        const name = shortcut ? `${sourceName} - 快捷方式.lnk` : sourceName;
        const dot = name.lastIndexOf(".");
        return {
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
      });
      items = [...items, ...created];
      return result({ items: created, warnings: [] } satisfies StagePathsResult);
    }
    case Commands.HoldPendingDrop: {
      const state = panelState(Number(args?.podId));
      state.mode = "ask";
      state.paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      state.visible = true;
      return result(undefined);
    }
    case Commands.RemoveItems: {
      const ids = new Set(Array.isArray(args?.ids) ? args.ids.map(Number) : []);
      items = items.filter((item) => !ids.has(item.id));
      return result(undefined);
    }
    case Commands.ExportItems: {
      const ids = Array.isArray(args?.ids) ? args.ids.map(Number) : [];
      if (args?.mode === "move") {
        const moved = new Set(ids);
        items = items.filter((item) => !moved.has(item.id));
      }
      return result({
        conflicts: [], completedIds: ids, skippedIds: [], staleIds: [], failed: [], warnings: [],
      } satisfies ExportResult);
    }
    case Commands.ReadThumbnail:
      return result(null);
    case Commands.ShowPanel:
      panelState(Number(args?.podId)).visible = true;
      return result(undefined);
    case Commands.TogglePanel: {
      // 与 Rust 侧 panel_toggle_action 一致：显示中点击即隐藏；未显示则以固定方式弹出。
      const state = panelState(Number(args?.podId));
      if (state.visible) {
        Object.assign(state, {
          mode: "list", paths: [], pinned: false, visible: false, draggingOut: false,
        });
      } else {
        state.visible = true;
        state.pinned = true;
      }
      return result(undefined);
    }
    case Commands.HidePanel:
      Object.assign(panelState(Number(args?.podId)), {
        mode: "list", paths: [], pinned: false, visible: false, draggingOut: false,
      });
      return result(undefined);
    case Commands.SetPanelMode: {
      const state = panelState(Number(args?.podId));
      state.mode = String(args?.mode ?? "list") as PanelMode;
      if (state.mode === "list") state.paths = [];
      return result(undefined);
    }
    case Commands.SetPanelPinned: {
      const state = panelState(Number(args?.podId));
      state.pinned = Boolean(args?.pinned);
      if (state.pinned) state.visible = true;
      return result(undefined);
    }
    case Commands.SetDraggingOut:
      panelState(Number(args?.podId)).draggingOut = Boolean(args?.dragging);
      return result(undefined);
    case Commands.PrepareDragCut: {
      const podId = Number(args?.podId);
      const paths = Array.isArray(args?.paths) ? args.paths.map(String) : [];
      if (!paths.length || new Set(paths).size !== paths.length) {
        throw new Error("剪切列表为空或包含重复路径");
      }
      if (paths.some((path) => !items.some(
        (item) => item.podId === podId && item.stagingPath === path,
      ))) {
        throw new Error("剪切路径不属于当前匣");
      }
      const token: DragCutToken = `mock-cut-${++cutSequence}`;
      cuts.set(token, { podId, paths: [...paths] });
      return result(token);
    }
    case Commands.FinalizeDragCut: {
      const token = String(args?.token ?? "") as DragCutToken;
      const cut = cuts.get(token);
      cuts.delete(token);
      if (!cut) throw new Error("剪切令牌无效或已被使用");
      const paths = new Set(cut.paths);
      items = items.filter((item) => item.podId !== cut.podId || !paths.has(item.stagingPath));
      return result(undefined);
    }
    case Commands.CancelDragCut:
      cuts.delete(String(args?.token ?? "") as DragCutToken);
      return result(undefined);
    case Commands.OpenContextMenu:
      // 浏览器预览没有菜单窗口：让面板走内嵌降级菜单。
      throw new Error("浏览器预览使用内嵌右键菜单");
    case Commands.WriteClipboardText: {
      const text = String(args?.text ?? "");
      void navigator.clipboard?.writeText(text).catch(() => {});
      return result(undefined);
    }
    default:
      // 浏览器预览没有原生窗口，这些命令直接返回成功。
      return result(undefined);
  }
}
