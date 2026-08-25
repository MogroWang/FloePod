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
  /** Removed from SQLite because the source was already absent; no target was produced. */
  staleIds: number[];
  failed: ExportIssue[];
  warnings: ExportIssue[];
}

export interface Pod {
  id: number;
  name: string;
  edge: Edge;
  /** Native monitor name; empty means the primary monitor. */
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
  /** Physical monitor rectangle and WebView scale factor. */
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
  /** Runtime metadata; excluded from persisted settings JSON. */
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

