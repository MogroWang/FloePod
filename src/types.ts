/** 与 Rust 侧 serde 对应的共享类型（camelCase 序列化） */

export type ItemKind = "file" | "folder" | "text" | "shortcut";
export type DropAction = "ask" | "copy" | "move" | "shortcut";
export type Edge = "top" | "right" | "bottom" | "left";
export type Material = "acrylic" | "plain";
export type ThemeMode = "system" | "light" | "dark";
export type ExportMode = "copy" | "move";
export type ConflictStrategy = "ask" | "overwrite" | "skip" | "rename";
/** 一次性剪切拖出令牌；只可 finalize 或 cancel 一次。 */
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

/** 暂存副作用已提交；warnings 表示目标有效但仍需人工检查的源清理问题。 */
export interface StagePathsResult {
  items: StagedItem[];
  warnings: StageWarning[];
}

/** 批量导出是逐项提交的；明确区分可安全重试的失败和已有副作用的警告。 */
export interface ExportResult {
  conflicts: string[];
  completedIds: number[];
  skippedIds: number[];
  /** 已从数据库清理、但目标目录没有产生文件的失效源条目。 */
  staleIds: number[];
  failed: ExportIssue[];
  warnings: ExportIssue[];
}

/** 一个「匣」：贴在屏幕边缘的独立暂存点 */
export interface Pod {
  id: number;
  name: string;
  edge: Edge;
  /** 显示器名；空串 = 主显示器 */
  monitor: string;
  /** 沿边缘位置 0 - 1 */
  offset: number;
  /** 保存文件夹 */
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
}

export interface StagedItem {
  id: number;
  podId: number;
  kind: ItemKind;
  /** 暂存文件夹内的绝对路径 */
  stagingPath: string;
  /** 原文件路径（copy/shortcut 时保留；text 为 null） */
  originalPath: string | null;
  name: string;
  ext: string | null;
  /** 字节数；folder 为 0 */
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
  /** OOBE 是否完成 */
  firstRunDone: boolean;
  autostart: boolean;
  hotkeys: Hotkeys;
  pods: Pod[];
  /** 只读信息：应用版本与数据目录 */
  version: string;
  dataDir: string;
}

export interface ModifierState {
  ctrl: boolean;
  shift: boolean;
  alt: boolean;
}

export interface PendingDrop {
  /** 待处理的原始路径 */
  paths: string[];
}

export interface ThumbnailPayload {
  mime: string;
  /** 图像字节 */
  bytes: number[];
}

export interface Bootstrap {
  settings: Settings;
  monitors: MonitorInfo[];
  version: string;
}

export type PanelMode = "list" | "ask" | "conflict";

/** Rust 侧单个面板的瞬时运行态快照。 */
export interface PanelState {
  mode: PanelMode;
  paths: string[];
  pinned: boolean;
  visible: boolean;
  draggingOut: boolean;
}
