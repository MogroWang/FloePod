export type ItemKind = "file" | "folder" | "text" | "shortcut";
export type DropAction = "ask" | "copy" | "move" | "shortcut";
export type Edge = "top" | "right" | "bottom" | "left";
export type Material = "acrylic" | "mica" | "blur" | "plain";
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
  /** 面板材质；与胶囊条材质独立设置。 */
  panelMaterial: Material;
  /** 面板填充色不透明度 0.1-1；作用于面板背景填充色。 */
  panelOpacity: number;
  /** 面板填充色（#RGB/#RRGGBB/#RRGGBBAA）；空字符串表示跟随主题表面色。 */
  panelColor: string;
  panelWidth: number;
  hoverDelayMs: number;
  /** 鼠标离开后是否自动收起面板。 */
  autoHide: boolean;
  /** 鼠标离开后到自动收起的延迟（毫秒）。 */
  autoHideDelayMs: number;
  dropAction: DropAction;
  enabled: boolean;
  /** 胶囊条短边宽度（逻辑像素）。 */
  barWidth: number;
  /** 胶囊条外角圆角半径；CSS 会自动收敛超过半宽的值。 */
  cornerRadius: number;
  /** 胶囊条边框颜色（#RGB/#RRGGBB/#RRGGBBAA）；空字符串表示跟随主题。 */
  borderColor: string;
  /** 边框不透明度 0-1，作用于 borderColor 或主题默认边框色。 */
  borderOpacity: number;
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

/** 自动屏蔽：配置的应用位于前台时暂时隐藏全部匣，离开前台后自动恢复。 */
export interface AutoBlock {
  enabled: boolean;
  /** 匹配前台应用的可执行文件名；允许完整路径，匹配时只取文件名。 */
  apps: string[];
}

export interface Settings {
  theme: ThemeMode;
  firstRunDone: boolean;
  autostart: boolean;
  hotkeys: Hotkeys;
  autoBlock: AutoBlock;
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
