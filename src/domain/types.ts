export type ItemKind = "file" | "folder" | "text" | "shortcut";
export type DropAction = "ask" | "copy" | "move" | "shortcut";
export type Edge = "top" | "right" | "bottom" | "left";
/** 窗口材质:亚克力 / 普通。云母已移除,存量配置迁移为亚克力。 */
export type Material = "acrylic" | "plain";
export type ThemeMode = "system" | "light" | "dark";
export type ExportMode = "copy" | "move";
export type ConflictStrategy = "ask" | "overwrite" | "skip" | "rename";
export type DragCutToken = string;

export interface PodRules {
  enabled: boolean;
  template: string;
  allowedExtensions: string[];
  nameContains: string;
  sourceFolder: string;
  maxSizeMb: number;
  renamePattern: string;
  subfolderPattern: string;
  duplicatePolicy: "allow" | "reject";
  checksumSidecar: boolean;
  expireDays: number;
  removeAfterExport: boolean;
}

export interface PodSecurity {
  enabled: boolean;
  requireWindowsHello: boolean;
  autoLockMinutes: number;
  retentionDays: number;
  cleanupAfterExport: boolean;
  suppressThumbnails: boolean;
  suppressIndex: boolean;
}

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

export interface OperationItem {
  id: number;
  itemId: number | null;
  name: string;
  sourcePath: string | null;
  targetPath: string | null;
  action: string;
  status: "completed" | "failed" | "skipped" | "stale" | string;
  error: string | null;
}

export interface OperationEntry {
  id: number;
  kind: string;
  podId: number | null;
  summary: string;
  status: string;
  createdAt: number;
  undoableUntil: number | null;
  undoneAt: number | null;
  undoable: boolean;
  retryable: boolean;
  items: OperationItem[];
}

export interface UndoResult {
  operationId: number;
  restored: number;
  failed: string[];
}

export interface RetryResult {
  operationId: number;
  kind: string;
  result: unknown;
}

export interface OperationPreview {
  title: string;
  details: string[];
  warnings: string[];
  requiresConfirmation: boolean;
}

export interface PrivacyIssue {
  path: string;
  code: string;
  severity: "low" | "medium" | "high" | string;
  message: string;
  canClean: boolean;
}

export interface PrivacyScanResult {
  filesScanned: number;
  issues: PrivacyIssue[];
  duplicates: string[][];
  disclaimer: string;
}

export interface CleanResult {
  source: string;
  output: string;
  removed: string[];
  warnings: string[];
}

export interface SafeExportResult {
  completed: CleanResult[];
  failed: string[];
}

export interface HandoffFile {
  relativePath: string;
  size: number;
  sha256: string;
  source: string;
  cleaned: string[];
}

export interface HandoffResult {
  directory: string;
  files: HandoffFile[];
  missing: string[];
  warnings: string[];
}

export interface VerifyResult {
  checked: number;
  valid: number;
  issues: Array<{ path: string; expected: string; actual: string | null }>;
}

export interface ItemAnnotation {
  tags: string[];
  note: string;
}

export interface SearchHit extends ItemAnnotation {
  item: StagedItem;
  snippet: string;
  matchedOn: string[];
}

export interface IndexResult {
  indexed: number;
  skipped: number;
  failures: string[];
  ocrAvailable: boolean;
}

export interface SecurityStatus {
  podId: number;
  sensitive: boolean;
  locked: boolean;
  efsEncrypted: boolean;
  expiresSoon: number;
}

export interface OrganizationPolicy {
  organizationName: string;
  disableMove: boolean;
  requireCopyDefault: boolean;
  requirePrivacyScan: boolean;
  lockRules: boolean;
  disableFulltextIndex: boolean;
  allowedDataRoots: string[];
  maximumHistoryDays: number;
  mandatoryRetentionDays: number;
  diagnosticIncludePaths: boolean;
  supportContact: string;
  managedHotkeys: Hotkeys | null;
  managedPods: Pod[];
}

export interface PolicyStatus {
  managed: boolean;
  source: string | null;
  policy: OrganizationPolicy;
}

export interface ExportedArtifact {
  path: string;
  records: number;
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
  /** 浮动面板材质；与边缘浮动条独立，边缘浮动条固定普通半透明、不提供材质设置。 */
  panelMaterial: Material;
  /** 浮动面板填充色不透明度 0.1-1；作用于浮动面板背景填充色。 */
  panelOpacity: number;
  /** 浮动面板填充色（#RGB/#RRGGBB/#RRGGBBAA）；空字符串表示跟随主题表面色。 */
  panelColor: string;
  panelWidth: number;
  hoverDelayMs: number;
  /** 是否允许悬停自动打开；关闭后仍可单击或用键盘打开。 */
  hoverOpen: boolean;
  /** 鼠标离开后是否自动收起浮动面板。 */
  autoHide: boolean;
  /** 鼠标离开后到自动收起的延迟（毫秒）。 */
  autoHideDelayMs: number;
  /** 隐匿模式：无交互超过延迟后边缘浮动条淡化隐去，鼠标靠近时再淡入。 */
  stealth: boolean;
  /** 隐匿模式下无交互到淡化隐去的延迟（毫秒）。 */
  stealthDelayMs: number;
  dropAction: DropAction;
  enabled: boolean;
  /** 边缘浮动条短边宽度（逻辑像素）。 */
  barWidth: number;
  /** 边缘浮动条外角圆角半径；CSS 会自动收敛超过半宽的值。 */
  cornerRadius: number;
  /** 边缘浮动条边框颜色（#RGB/#RRGGBB/#RRGGBBAA）；空字符串表示跟随主题。 */
  borderColor: string;
  /** 边框不透明度 0-1，作用于 borderColor 或主题默认边框色。 */
  borderOpacity: number;
  rules: PodRules;
  security: PodSecurity;
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
  lockSensitive: string;
}

/** 自动屏蔽：配置的应用位于前台时暂时隐藏全部匣，离开前台后自动恢复。 */
export interface AutoBlock {
  enabled: boolean;
  /** 匹配前台应用的可执行文件名；允许完整路径，匹配时只取文件名。 */
  apps: string[];
}

export interface AccessibilitySettings {
  enabled: boolean;
  /** WebView 内容缩放，1 = 100%，2 = 200%。 */
  scale: number;
  highContrast: boolean;
  reduceTransparency: boolean;
  reduceMotion: boolean;
  simpleLanguage: boolean;
  confirmDangerous: boolean;
  sendToMenu: boolean;
}

export interface Settings {
  theme: ThemeMode;
  firstRunDone: boolean;
  autostart: boolean;
  hotkeys: Hotkeys;
  autoBlock: AutoBlock;
  accessibility: AccessibilitySettings;
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
