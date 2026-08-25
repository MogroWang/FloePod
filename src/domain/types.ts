export type ItemKind = "file" | "folder" | "text" | "shortcut";
export type DropAction = "ask" | "copy" | "move" | "shortcut";
export type Edge = "top" | "right" | "bottom" | "left";
export type Material = "acrylic" | "plain";
export type ThemeMode = "system" | "light" | "dark";
export type ExportMode = "copy" | "move";
export type ConflictStrategy = "ask" | "overwrite" | "skip" | "rename";
export type DragCutToken = string;

export interface ExportIssue {
  id: number;
  name: string;
  error: string;
}

export interface StageWarning {
  name: string;
  error: string;
}

export interface StagePathsResult {
  items: StagedItem[];
  warnings: StageWarning[];
}

export interface ExportResult {
  conflicts: string[];
  completedIds: number[];
  skippedIds: number[];
  /** 源文件已不存在，数据库记录已清理，但没有生成目标文件。 */
  staleIds: number[];
  failed: ExportIssue[];
  warnings: ExportIssue[];
}

export interface Pod {
  id: number;
  name: string;
  edge: Edge;
  /** 系统显示器名称；空字符串表示主显示器。 */
  monitor: string;
  offset: number;
  stagingFolder: string;
  opacity: number;
  material: Material;
  panelWidth: number;
  hoverDelayMs: number;
  dropAction: DropAction;
  enabled: boolean;
}

export interface MonitorInfo {
  name: string;
  label: string;
  primary: boolean;
  /** 显示器物理坐标和 WebView 缩放比例。 */
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
}

export interface StagedItem {
  id: number;
  podId: number;
  kind: ItemKind;
  stagingPath: string;
  originalPath: string | null;
  name: string;
  ext: string | null;
  size: number;
  createdAt: number;
}

export interface Hotkeys {
  toggleBar: string;
  collectClipboard: string;
  openPanel: string;
}

export interface Settings {
  theme: ThemeMode;
  firstRunDone: boolean;
  autostart: boolean;
  hotkeys: Hotkeys;
  pods: Pod[];
  /** 运行时信息，不写入设置文件。 */
  version: string;
  dataDir: string;
}

export interface ModifierState {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export interface ThumbnailPayload {
  mime: string;
  bytes: number[];
}

export interface Bootstrap {
  settings: Settings;
  monitors: MonitorInfo[];
  version: string;
}

export type PanelMode = "list" | "ask" | "conflict";

export interface PanelState {
  mode: PanelMode;
  paths: string[];
  pinned: boolean;
  visible: boolean;
  draggingOut: boolean;
}
