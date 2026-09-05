<script setup lang="ts">
/**
 * 设置窗口：OOBE 首启引导 / 常规 / 匣管理 / 快捷键 / 关于。
 * 所有修改即时保存（save -> Rust 持久化并广播 settings_changed）。
 * 排版原则：单一强调色、层级靠字号与留白、分区标题建立可扫视结构、
 * 进入/切换动画统一使用出程缓动（--ease-out）。
 */
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useToast } from "@/composables/useToast";
import { normalizeWindowsPathKey } from "@/domain/settings";
import type {
  AccessibilitySettings,
  AutoBlock,
  DropAction,
  Edge,
  Material,
  Pod,
  ThemeMode,
} from "@/domain/types";
import { ipc } from "@/ipc/client";
import { BROWSER_PREVIEW_STAGING_ROOT } from "@/lib/env";
import { useSettingsStore } from "@/stores/settings";
import SegmentedControl from "@/components/SegmentedControl.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import SettingsRow from "@/components/SettingsRow.vue";
import HotkeyRecorder from "@/components/HotkeyRecorder.vue";
import BrandMark from "@/components/BrandMark.vue";
import RangeSlider from "@/components/RangeSlider.vue";
import PodEdgeDiagram from "@/components/PodEdgeDiagram.vue";
import SafetyCenter from "@/components/SafetyCenter.vue";
import PodRulesEditor from "@/components/PodRulesEditor.vue";
import SearchCenter from "@/components/SearchCenter.vue";
import PodSecurityEditor from "@/components/PodSecurityEditor.vue";
import OrganizationCenter from "@/components/OrganizationCenter.vue";

const settingsStore = useSettingsStore();
const s = computed(() => settingsStore.settings);
const monitors = computed(() => settingsStore.monitors);

const page = ref<"general" | "safety" | "advanced" | "pods" | "hotkeys" | "about">("general");
const { toast, showToast, disposeToast } = useToast(2400);
const hotkeyError = ref("");
const loading = ref(true);
const loadError = ref("");
const autostartBusy = ref(false);
const deletingPodIds = reactive(new Set<number>());
const podEnabledBusyIds = reactive(new Set<number>());
let settingsSaveTail: Promise<void> = Promise.resolve();
const podSaveTails = new Map<number, Promise<void>>();
let hotkeySaveRevision = 0;

/** 匣管理：多个匣时用列表选中要编辑的匣，而不是把所有匣的设置都铺开。 */
const selectedPodId = ref<number | null>(null);
const selectedPod = computed<Pod | null>(() => {
  const pods = s.value?.pods ?? [];
  return pods.find((pod) => pod.id === selectedPodId.value) ?? pods[0] ?? null;
});
/** 单元素列表：模板里 v-for 出的 `pod` 恒为非空，事件回调无需逐处判空。 */
const selectedPodList = computed<Pod[]>(() => (selectedPod.value ? [selectedPod.value] : []));

function selectPod(id: number) {
  selectedPodId.value = id;
}

const oobeDone = ref(false);
const firstRun = computed(
  () =>
    !oobeDone.value &&
    !!s.value &&
    (s.value.pods.length === 0 || !s.value.firstRunDone),
);
const oobeStep = ref(1);
const oobe = ref({
  name: "我的匣",
  edge: "left" as Edge,
  monitor: "",
  folder: "",
  theme: "system" as ThemeMode,
  opacity: 1,
  material: "acrylic" as Material,
});
const oobeBusy = ref(false);
const oobeCreateStarted = ref(false);
let oobeCreatePromise: Promise<Pod> | null = null;
let oobeCreatedPod: Pod | null = null;

const EDGES: { value: Edge; label: string }[] = [
  { value: "top", label: "上" },
  { value: "bottom", label: "下" },
  { value: "left", label: "左" },
  { value: "right", label: "右" },
];

const DROP_ACTIONS: { value: DropAction; label: string }[] = [
  { value: "ask", label: "询问" },
  { value: "copy", label: "复制" },
  { value: "move", label: "移动" },
  { value: "shortcut", label: "快捷方式" },
];

const MATERIALS: { value: Material; label: string }[] = [
  { value: "acrylic", label: "亚克力" },
  { value: "plain", label: "普通" },
];

const THEMES: { value: ThemeMode; label: string }[] = [
  { value: "system", label: "跟随系统" },
  { value: "light", label: "浅色" },
  { value: "dark", label: "深色" },
];

/** 侧栏导航图标（单线条，随 currentColor）。
 *  v-html 的内容是这份静态开发者常量，不含任何运行时输入；
 *  若未来需要动态图标，必须改用组件渲染而非拼接字符串。 */
const NAV_ICONS: Record<string, string> = {
  general:
    '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M2 14h4M10 8h4M18 16h4"/>',
  safety:
    '<circle cx="12" cy="4.5" r="2"/><path d="M4.5 9c2.5.9 5 1.3 7.5 1.3S17 9.9 19.5 9M12 10.3V15m0 0-3 6m3-6 3 6"/>',
  advanced:
    '<path d="M5 8h8M17 8h2M5 16h2M11 16h8"/><circle cx="15" cy="8" r="2"/><circle cx="9" cy="16" r="2"/>',
  pods: '<rect x="3.5" y="3.5" width="17" height="17" rx="3"/><path d="M12 8v8M8 12h8"/>',
  hotkeys:
    '<rect x="3" y="6.5" width="18" height="11" rx="2"/><path d="M7.5 12h.01M12 12h.01M16.5 12h.01M10 15h4"/>',
  about: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>',
};

function monitorLabel(pod: Pod): string {
  if (!pod.monitor) return "主显示器";
  return monitors.value.find((m) => m.name === pod.monitor)?.label ?? pod.monitor;
}

function appLog(msg: string) {
  void ipc.appLog(msg).catch((err) => console.warn("app log failed", err));
}

async function checkUpdates() {
  try {
    await openUrl("https://github.com/MogroWang/FloePod/releases/latest");
  } catch (error) {
    showToast(`无法打开版本页面：${String(error)}`);
  }
}

/** 带超时的等待：Promise 15 秒不返回则抛错，避免永远卡在「创建中」 */
function withTimeout<T>(p: Promise<T>, label: string, ms = 15000): Promise<T> {
  return new Promise<T>((resolve, reject) => {
    const t = window.setTimeout(() => reject(new Error(`${label} 超时（${ms}ms）`)), ms);
    p.then(
      (v) => {
        window.clearTimeout(t);
        resolve(v);
      },
      (e) => {
        window.clearTimeout(t);
        reject(e);
      },
    );
  });
}

type SettingsPatch = Parameters<typeof settingsStore.save>[0];
type SettingsPatchSource = SettingsPatch | (() => SettingsPatch);

function enqueueSettingsSave(source: SettingsPatchSource) {
  // 保存轮到执行时再计算新值，连续编辑才能基于上一次保存结果合并。
  const request = settingsSaveTail.then(() =>
    settingsStore.save(typeof source === "function" ? source() : source),
  );
  settingsSaveTail = request.then(
    () => undefined,
    () => undefined,
  );
  return request;
}

async function save(patch: SettingsPatch): Promise<boolean> {
  try {
    await enqueueSettingsSave(patch);
    return true;
  } catch (err) {
    console.error(err);
    showToast("保存失败，请重试");
    return false;
  }
}

function saveAccessibility(patch: Partial<AccessibilitySettings>) {
  const current = s.value?.accessibility;
  if (!current) return;
  void save({ accessibility: { ...current, ...patch } });
}

async function saveAutostart(enabled: boolean) {
  if (autostartBusy.value) return;
  autostartBusy.value = true;
  try {
    await save({ autostart: enabled });
  } finally {
    autostartBusy.value = false;
  }
}

async function pickFolder(): Promise<string | null> {
  if (!ipc.inTauri) return BROWSER_PREVIEW_STAGING_ROOT;
  try {
    const dir = await open({ directory: true, multiple: false, title: "选择暂存文件夹" });
    return typeof dir === "string" ? dir : null;
  } catch (err) {
    console.error("folder picker failed", err);
    showToast("无法打开文件夹选择器");
    return null;
  }
}

async function openPodFolder(pod: Pod) {
  if (!pod.stagingFolder) return;
  if (!ipc.inTauri) {
    showToast(`浏览器预览：${pod.stagingFolder}`);
    return;
  }
  try {
    // 打开文件夹走后端校验命令，WebView 不直接持有 openPath 能力。
    await ipc.openPodFolder(pod.id);
  } catch (err) {
    console.error("open pod folder failed", err);
    showToast("无法打开暂存文件夹");
  }
}

async function chooseOobeFolder() {
  const folder = await pickFolder();
  if (folder) oobe.value.folder = folder;
}

async function changePodFolder(pod: Pod) {
  const folder = await pickFolder();
  if (folder) await savePod(pod.id, { stagingFolder: folder });
}

function enqueuePodSave(id: number, patch: Partial<Pod>) {
  const previous = podSaveTails.get(id) ?? Promise.resolve();
  const request = previous.then(async () => {
    const updated = await ipc.updatePod(id, patch);
    await settingsStore.refreshPods();
    return updated;
  });
  const tail = request.then(
    () => undefined,
    () => undefined,
  );
  podSaveTails.set(id, tail);
  void tail.then(() => {
    if (podSaveTails.get(id) === tail) podSaveTails.delete(id);
  });
  return request;
}

async function savePod(id: number, patch: Partial<Pod>): Promise<boolean> {
  try {
    await enqueuePodSave(id, patch);
    return true;
  } catch (err) {
    console.error(err);
    await settingsStore.refreshPods().catch((refreshError) => {
      console.error("pod rollback refresh failed", refreshError);
    });
    showToast("保存失败，请重试");
    return false;
  }
}

async function savePodEnabled(pod: Pod, enabled: boolean) {
  if (podEnabledBusyIds.has(pod.id)) return;
  podEnabledBusyIds.add(pod.id);
  try {
    await savePod(pod.id, { enabled });
  } finally {
    podEnabledBusyIds.delete(pod.id);
  }
}

type PodNumberField =
  | "offset"
  | "opacity"
  | "panelOpacity"
  | "panelWidth"
  | "hoverDelayMs"
  | "autoHideDelayMs"
  | "stealthDelayMs"
  | "barWidth"
  | "cornerRadius"
  | "borderOpacity";
const podNumberDrafts = reactive<Record<number, Partial<Record<PodNumberField, number>>>>({});

function podNumberValue(pod: Pod, field: PodNumberField): number {
  return podNumberDrafts[pod.id]?.[field] ?? pod[field];
}

function previewPodNumber(id: number, field: PodNumberField, value: number) {
  const draft = (podNumberDrafts[id] ??= {});
  draft[field] = value;
}

async function commitPodNumber(pod: Pod, field: PodNumberField, value: number) {
  previewPodNumber(pod.id, field, value);
  await savePod(pod.id, { [field]: value });
  // 请求排队期间用户可能已经输入新值，不能把它一起清空。
  if (podNumberDrafts[pod.id]?.[field] === value) {
    delete podNumberDrafts[pod.id]?.[field];
    if (Object.keys(podNumberDrafts[pod.id] ?? {}).length === 0) delete podNumberDrafts[pod.id];
  }
}

/** 重命名：默认只读展示，点编辑图标后输入框才出现。 */
const renamingPodId = ref<number | null>(null);
const renameDraft = ref("");
const renameInput = ref<HTMLInputElement | null>(null);

function startRename(pod: Pod) {
  renamingPodId.value = pod.id;
  renameDraft.value = pod.name;
  void nextTick(() => {
    const input = renameInput.value;
    if (!input) return;
    input.focus();
    input.select();
  });
}

function cancelRename() {
  renamingPodId.value = null;
}

async function commitRename() {
  const id = renamingPodId.value;
  if (id == null) return;
  renamingPodId.value = null;
  const pod = settingsStore.pod(id);
  const value = renameDraft.value.trim();
  if (!pod || !value || value === pod.name) return;
  await savePod(id, { name: value });
}

/** 颜色字段：空字符串表示跟随主题；3 位十六进制展开成 6 位供取色器回显。
 *  绑定 @change（确认选色后才触发），选色过程中不会实时改匣的外观。 */
const HEX_COLOR_FALLBACK = "#ffffff";

function podHexColorValue(raw: string): string {
  const triple = /^#([0-9a-fA-F])([0-9a-fA-F])([0-9a-fA-F])$/.exec(raw);
  if (triple) {
    return `#${triple[1]}${triple[1]}${triple[2]}${triple[2]}${triple[3]}${triple[3]}`;
  }
  return raw || HEX_COLOR_FALLBACK;
}

/** 颜色草稿：选色过程只更新草稿，点「确定」后才保存并生效。 */
const colorDrafts = reactive<
  Record<number, Partial<Record<"borderColor" | "panelColor", string>>>
>({});

function colorDraftValue(pod: Pod, field: "borderColor" | "panelColor"): string {
  return colorDrafts[pod.id]?.[field] ?? pod[field];
}

function previewPodColor(pod: Pod, field: "borderColor" | "panelColor", value: string) {
  (colorDrafts[pod.id] ??= {})[field] = value;
}

/** 草稿与已保存值（归一化为 6 位 hex）不同时才显示确认按钮。 */
function hasColorDraft(pod: Pod, field: "borderColor" | "panelColor"): boolean {
  const draft = colorDrafts[pod.id]?.[field];
  return draft != null && draft !== podHexColorValue(pod[field]);
}

async function confirmPodColor(pod: Pod, field: "borderColor" | "panelColor") {
  const value = colorDrafts[pod.id]?.[field];
  if (!value) return;
  delete colorDrafts[pod.id]?.[field];
  await savePod(pod.id, { [field]: value });
}

async function clearPodColor(pod: Pod, field: "borderColor" | "panelColor") {
  delete colorDrafts[pod.id]?.[field];
  if (pod[field]) await savePod(pod.id, { [field]: "" });
}

async function commitPodMonitor(pod: Pod, event: Event) {
  const select = event.target as HTMLSelectElement;
  const value = select.value;
  const saved = await savePod(pod.id, { monitor: value });
  if (!saved && select.value === value) {
    select.value = settingsStore.pod(pod.id)?.monitor ?? pod.monitor;
  }
}

/** 新建匣弹窗：先询问名称与文件夹位置，确认后才创建。 */
const addDialogOpen = ref(false);
const addPodCreating = ref(false);
const addDraft = reactive({ name: "", folder: "" });

function openAddDialog() {
  addDraft.name = `匣 ${(s.value?.pods.length ?? 0) + 1}`;
  addDraft.folder = "";
  addDialogOpen.value = true;
}

function cancelAddDialog() {
  if (addPodCreating.value) return;
  addDialogOpen.value = false;
}

async function chooseAddFolder() {
  const folder = await pickFolder();
  if (folder) addDraft.folder = folder;
}

async function confirmAddPod() {
  if (addPodCreating.value) return;
  const folder = addDraft.folder.trim();
  if (!folder) {
    showToast("请先选择保存文件夹");
    return;
  }
  addPodCreating.value = true;
  try {
    const n = s.value?.pods.length ?? 0;
    const edge = (["left", "right", "top", "bottom"] as Edge[])[n % 4];
    const pod = await ipc.createPod({
      name: addDraft.name.trim() || `匣 ${n + 1}`,
      edge,
      monitor: "",
      offset: 0.5,
      stagingFolder: folder,
      opacity: 1,
      panelMaterial: "acrylic",
      panelOpacity: 1,
      panelColor: "",
      panelWidth: 380,
      hoverDelayMs: 120,
      hoverOpen: true,
      autoHide: true,
      autoHideDelayMs: 320,
      stealth: false,
      stealthDelayMs: 3000,
      dropAction: "ask",
      enabled: true,
      barWidth: 44,
      cornerRadius: 22,
      borderColor: "",
      borderOpacity: 1,
      rules: {
        enabled: false,
        template: "manual",
        allowedExtensions: [],
        nameContains: "",
        sourceFolder: "",
        maxSizeMb: 0,
        renamePattern: "{name}",
        subfolderPattern: "",
        duplicatePolicy: "allow",
        checksumSidecar: false,
        expireDays: 0,
        removeAfterExport: false,
      },
      security: {
        enabled: false,
        requireWindowsHello: true,
        autoLockMinutes: 10,
        retentionDays: 0,
        cleanupAfterExport: false,
        suppressThumbnails: true,
        suppressIndex: true,
      },
    });
    await settingsStore.refreshPods();
    selectedPodId.value = pod.id;
    addDialogOpen.value = false;
    showToast("已创建新匣");
  } catch (err) {
    console.error(err);
    showToast("创建失败，请重试");
  } finally {
    addPodCreating.value = false;
  }
}

/** 删除匣：先选择「仅移除匣」还是「连暂存文件夹一起删除」。 */
const deleteDialog = ref<{ pod: Pod } | null>(null);

function requestRemovePod(pod: Pod) {
  deleteDialog.value = { pod };
}

function cancelRemovePod() {
  deleteDialog.value = null;
}

async function doRemovePod(pod: Pod, mode: "keep" | "folder", done: string) {
  if (deletingPodIds.has(pod.id)) return;
  deletingPodIds.add(pod.id);
  try {
    await ipc.deletePod(pod.id, mode);
    await settingsStore.refreshPods();
    showToast(done);
  } catch (err) {
    console.error(err);
    showToast("删除失败，请重试");
  } finally {
    deletingPodIds.delete(pod.id);
  }
}

async function removePodKeepingFiles() {
  const target = deleteDialog.value?.pod;
  if (!target) return;
  deleteDialog.value = null;
  await doRemovePod(target, "keep", "已移除匣（暂存文件夹与文件保留）");
}

/** 连文件夹删除是重操作：选择后再弹一次原生确认。 */
async function removePodWithFolder() {
  const target = deleteDialog.value?.pod;
  if (!target) return;
  deleteDialog.value = null;
  const message = `将把暂存文件夹「${target.stagingFolder}」连同全部内容移入回收站，并删除匣「${target.name}」。确定继续？`;
  const ok = ipc.inTauri
    ? await ask(message, { title: "删除匣与文件", kind: "warning" })
    : window.confirm(message);
  if (!ok) return;
  await doRemovePod(target, "folder", "已删除匣（暂存文件夹已移入回收站）");
}

function oobePodConfig(): Omit<Pod, "id"> {
  return {
    name: oobe.value.name || "我的匣",
    edge: oobe.value.edge,
    monitor: oobe.value.monitor,
    offset: 0.5,
    stagingFolder: oobe.value.folder,
    opacity: Number(oobe.value.opacity),
    panelMaterial: oobe.value.material,
    panelOpacity: Number(oobe.value.opacity),
    panelColor: "",
    panelWidth: 380,
    hoverDelayMs: 120,
    hoverOpen: true,
    autoHide: true,
    autoHideDelayMs: 320,
    stealth: false,
    stealthDelayMs: 3000,
    dropAction: "ask",
    enabled: true,
    barWidth: 44,
    cornerRadius: 22,
    borderColor: "",
    borderOpacity: 1,
    rules: {
      enabled: false,
      template: "manual",
      allowedExtensions: [],
      nameContains: "",
      sourceFolder: "",
      maxSizeMb: 0,
      renamePattern: "{name}",
      subfolderPattern: "",
      duplicatePolicy: "allow",
      checksumSidecar: false,
      expireDays: 0,
      removeAfterExport: false,
    },
    security: {
      enabled: false,
      requireWindowsHello: true,
      autoLockMinutes: 10,
      retentionDays: 0,
      cleanupAfterExport: false,
      suppressThumbnails: true,
      suppressIndex: true,
    },
  };
}

function findOobePodByFolder(): Pod | undefined {
  const folderKey = normalizeWindowsPathKey(oobe.value.folder);
  if (!folderKey) return undefined;
  return s.value?.pods.find(
    (pod) => normalizeWindowsPathKey(pod.stagingFolder) === folderKey,
  );
}

function ensureOobePod(): Promise<Pod> {
  if (oobeCreatedPod) return Promise.resolve(oobeCreatedPod);
  if (oobeCreatePromise) return oobeCreatePromise;

  // 若上次已经创建匣但未及时写入 firstRunDone，重试时复用该匣。
  const existing = findOobePodByFolder();
  if (existing) {
    oobeCreatedPod = existing;
    oobeCreateStarted.value = true;
    return Promise.resolve(existing);
  }

  oobeCreateStarted.value = true;
  // 只有首次设置允许幂等重试；普通新建仍需报告目录重复。
  const request = ipc.createPod(oobePodConfig(), true);
  let operation!: Promise<Pod>;
  operation = request.then(
    (pod) => {
      oobeCreatedPod = pod;
      return pod;
    },
    async (err) => {
      // 连接失败不代表后端没有提交；允许重试前先同步一次，避免重复创建。
      await settingsStore.refreshPods().catch((refreshError) => {
        console.error("OOBE create reconciliation failed", refreshError);
      });
      const existingAfterFailure = findOobePodByFolder();
      if (existingAfterFailure) {
        oobeCreatedPod = existingAfterFailure;
        return existingAfterFailure;
      }
      // 较早的超时操作可能晚于重试结束，不能清除新请求的忙碌状态。
      if (oobeCreatePromise === operation) {
        oobeCreatePromise = null;
        oobeCreateStarted.value = false;
      }
      throw err;
    },
  );
  oobeCreatePromise = operation;
  return operation;
}

async function finishOobe() {
  if (oobeBusy.value) return;
  if (!oobe.value.folder) {
    showToast("请先选择保存文件夹");
    return;
  }
  oobeBusy.value = true;
  appLog("finishOobe 开始");
  try {
    appLog("finishOobe: 获取或创建 Pod");
    await withTimeout(ensureOobePod(), "createPod");
    appLog("finishOobe: createPod 完成");
    await withTimeout(
      ipc.saveSettings({ theme: oobe.value.theme, firstRunDone: true }),
      "saveSettings",
    );
    appLog("finishOobe: saveSettings 完成");
    await withTimeout(settingsStore.refreshPods(), "refreshPods");
    appLog("finishOobe: refreshPods 完成");
    oobeDone.value = true; // 兜底：确保向导退出
    page.value = "pods";
  } catch (err) {
    console.error("finishOobe failed", err);
    appLog(`finishOobe 失败: ${err}`);
    // 超时不会终止底层请求；先同步再允许重试，让前后两次请求落到同一个目录。
    if (String(err).includes("createPod 超时")) {
      await settingsStore.refreshPods().catch((refreshError) => {
        console.error("OOBE timeout reconciliation failed", refreshError);
      });
      const existingAfterTimeout = findOobePodByFolder();
      if (existingAfterTimeout) oobeCreatedPod = existingAfterTimeout;
      else {
        oobeCreatePromise = null;
        oobeCreateStarted.value = false;
      }
    }
    showToast(`创建失败：${err}`);
  } finally {
    oobeBusy.value = false;
  }
}

/** OOBE 第二步：未选文件夹时不允许进入下一步 */
function nextFromStep2() {
  if (!oobe.value.folder) {
    showToast("请先选择保存文件夹");
    return;
  }
  oobeStep.value = 3;
}

async function saveHotkey(
  key: "toggleBar" | "collectClipboard" | "openPanel" | "lockSensitive",
  combo: string,
) {
  const revision = ++hotkeySaveRevision;
  hotkeyError.value = "";
  try {
    await enqueueSettingsSave(() => ({
      hotkeys: { ...s.value!.hotkeys, [key]: combo },
    }));
    if (revision === hotkeySaveRevision) hotkeyError.value = "";
  } catch (err) {
    if (revision !== hotkeySaveRevision) return;
    hotkeyError.value = `快捷键「${combo}」注册失败，可能与其他软件冲突`;
    showToast(hotkeyError.value);
  }
}

async function resetHotkeys() {
  const revision = ++hotkeySaveRevision;
  try {
    const defaults = await ipc.getHotkeyDefaults();
    // 获取默认值本身也是异步的；若此时用户已经录入了更新的快捷键，旧的
    // “重置”意图不能晚到并排在新值之后覆盖它。
    if (revision !== hotkeySaveRevision) return;
    await enqueueSettingsSave({ hotkeys: defaults });
    if (revision === hotkeySaveRevision) {
      hotkeyError.value = "";
      showToast("已恢复默认快捷键");
    }
  } catch (err) {
    if (revision !== hotkeySaveRevision) return;
    console.error("reset hotkeys failed", err);
    showToast("重置失败，请重试");
  }
}

async function winMinimize() {
  if (!ipc.inTauri) return;
  try {
    await getCurrentWindow().minimize();
  } catch (err) {
    console.error("minimize settings failed", err);
  }
}

async function winClose() {
  if (!ipc.inTauri) return;
  try {
    await getCurrentWindow().hide();
  } catch (err) {
    console.error("hide settings failed", err);
  }
}

async function quitApp() {
  try {
    await ipc.quitApp();
  } catch (err) {
    console.error("quit app failed", err);
    showToast("退出失败，请从托盘重试");
  }
}

// ---- 自动屏蔽 ----

const blockAppDraft = ref("");

async function saveAutoBlock(next: AutoBlock) {
  await save({
    autoBlock: {
      enabled: next.enabled,
      apps: next.apps.map((app) => app.trim()).filter((app) => app.length > 0),
    },
  });
}

/** 模板回调里无法收窄 s 的可空性，改从这里取当前配置。 */
function toggleAutoBlock(enabled: boolean) {
  const current = s.value?.autoBlock;
  if (!current) return;
  void saveAutoBlock({ ...current, enabled });
}

/** 与 Rust 侧 exe_matches 对齐：取文件名、小写化、补齐 .exe 后缀后比较。 */
function blockAppKey(raw: string): string {
  const trimmed = raw.trim().replace(/^"+|"+$/g, "");
  const parts = trimmed.split(/[\\/]/).filter((part) => part.length > 0);
  const name = (parts[parts.length - 1] ?? trimmed).toLowerCase();
  return name.endsWith(".exe") ? name : `${name}.exe`;
}

function addBlockApp(raw: string) {
  const value = raw.trim().replace(/^"+|"+$/g, "");
  blockAppDraft.value = "";
  if (!value) return;
  const current = s.value?.autoBlock;
  if (!current) return;
  if (current.apps.some((app) => blockAppKey(app) === blockAppKey(value))) {
    showToast("该应用已在列表中");
    return;
  }
  void saveAutoBlock({ enabled: current.enabled, apps: [...current.apps, value] });
}

function removeBlockApp(index: number) {
  const current = s.value?.autoBlock;
  if (!current) return;
  void saveAutoBlock({
    enabled: current.enabled,
    apps: current.apps.filter((_, i) => i !== index),
  });
}

async function pickBlockApp() {
  if (!ipc.inTauri) {
    showToast("浏览器预览：请手动输入进程名");
    return;
  }
  try {
    const file = await open({
      multiple: false,
      title: "选择要屏蔽的应用",
      filters: [{ name: "应用程序", extensions: ["exe"] }],
    });
    if (typeof file === "string" && file) addBlockApp(file);
  } catch (err) {
    console.error("pick block app failed", err);
    showToast("无法打开文件选择器");
  }
}

async function loadSettings() {
  loading.value = true;
  loadError.value = "";
  try {
    // 先订阅再取快照；事件若在 getBootstrap 期间到达，store 的 revision
    // 会阻止旧快照覆盖事件。
    await settingsStore.listenChanges().catch((err) => {
      console.error("settings listener failed", err);
      showToast("设置实时同步不可用");
    });
    await settingsStore.load();
    appLog(
      `SettingsWindow mounted | firstRun=${firstRun.value} | pods=${s.value?.pods.length ?? 0} | firstRunDone=${s.value?.firstRunDone}`,
    );
    if (firstRun.value) {
      // 若上次已创建首个匣但未写入 firstRunDone，直接继续该匣，避免重启后重复创建。
      const existing = s.value?.pods[0];
      if (existing) {
        oobeCreatedPod = existing;
        oobeCreateStarted.value = true;
        oobe.value = {
          ...oobe.value,
          name: existing.name,
          edge: existing.edge,
          monitor: existing.monitor,
          folder: existing.stagingFolder,
          opacity: existing.opacity,
          material: existing.panelMaterial,
        };
        oobeStep.value = 3;
      } else {
        oobeStep.value = 1;
      }
    }
    else page.value = "general";
  } catch (err) {
    console.error("settings initialization failed", err);
    loadError.value = "设置加载失败，请检查数据目录后重试。";
  } finally {
    loading.value = false;
  }
}

onMounted(() => void loadSettings());
onBeforeUnmount(() => disposeToast());

const PAGES = [
  { id: "general", label: "常规" },
  { id: "pods", label: "匣" },
  { id: "safety", label: "辅助功能" },
  { id: "advanced", label: "高级设置" },
  { id: "hotkeys", label: "快捷键" },
  { id: "about", label: "关于" },
] as const;
</script>

<template>
  <div class="settings-root">
    <div class="titlebar" data-tauri-drag-region>
      <div class="titlebar-title" data-tauri-drag-region>浮匣 FloePod 设置界面</div>
      <div class="titlebar-controls">
        <button type="button" class="tb-btn" title="最小化" @click="winMinimize">
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <path d="M2 6h8" />
          </svg>
        </button>
        <button type="button" class="tb-btn close" title="关闭" @click="winClose">
          <svg width="12" height="12" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
            <path d="m3 3 6 6M9 3l-6 6" />
          </svg>
        </button>
      </div>
    </div>

    <div v-if="loading" class="load-state" role="status" aria-live="polite">
      正在加载设置…
    </div>
    <div v-else-if="loadError" class="load-state error-state" role="alert">
      <p>{{ loadError }}</p>
      <button type="button" class="btn primary" @click="loadSettings">重试</button>
    </div>

    <template v-else-if="s">
    <div v-if="firstRun" class="oobe">
      <div class="oobe-card">
        <template v-if="oobeStep === 1">
          <BrandMark :size="56" class="oobe-brand" />
          <h1 class="oobe-title">欢迎使用浮匣</h1>
          <p class="oobe-text">
            浮匣是贴在屏幕边缘的暂存小工具：把任何文件拖到匣上，松手即可暂存；
            需要时再把文件从匣的窗口拖出去继续使用。
          </p>
          <p class="oobe-text dim">现在先创建一个「匣」吧。</p>
          <button type="button" class="btn primary" @click="oobeStep = 2">开始</button>
        </template>

        <template v-else-if="oobeStep === 2">
          <h2 class="oobe-step-title">创建你的匣</h2>
          <div class="oobe-form">
            <label class="field">
              <span>名称</span>
              <input v-model="oobe.name" class="input" maxlength="12" placeholder="我的匣" />
            </label>
            <label class="field">
              <span>贴在屏幕哪一边</span>
              <SegmentedControl :options="EDGES" v-model="oobe.edge" />
            </label>
            <label class="field">
              <span>显示器</span>
              <select v-model="oobe.monitor" class="input">
                <option value="">主显示器</option>
                <option v-for="m in monitors" :key="m.name" :value="m.name">{{ m.label }}</option>
              </select>
            </label>
            <div class="field">
              <span>保存文件夹</span>
              <div class="folder-line">
                <input :value="oobe.folder" class="input mono" readonly placeholder="选择存放暂存文件的文件夹" />
                <button type="button" class="btn" @click="chooseOobeFolder">
                  选择…
                </button>
              </div>
            </div>
          </div>
          <div class="oobe-actions">
            <button type="button" class="btn ghost" @click="oobeStep = 1">上一步</button>
            <button type="button" class="btn primary" :disabled="!oobe.folder" @click="nextFromStep2">下一步</button>
          </div>
        </template>

        <template v-else>
          <h2 class="oobe-step-title">个性化</h2>
          <div class="oobe-form">
            <label class="field">
              <span>主题</span>
              <SegmentedControl :options="THEMES" v-model="oobe.theme" />
            </label>
            <label class="field">
              <span>不透明度</span>
              <div class="slider-line">
                <RangeSlider
                  :value="oobe.opacity"
                  :min="0.1"
                  :max="1"
                  :step="0.01"
                  aria-label="不透明度"
                  @update:value="(v) => (oobe.opacity = v)"
                />
                <span class="fval">{{ Math.round(oobe.opacity * 100) }}%</span>
              </div>
            </label>
            <label class="field">
              <span>浮动面板材质</span>
              <SegmentedControl :options="MATERIALS" v-model="oobe.material" />
            </label>
          </div>
          <div class="oobe-actions">
            <button
              type="button"
              class="btn ghost"
              :disabled="oobeBusy || oobeCreateStarted"
              @click="oobeStep = 2"
            >
              上一步
            </button>
            <button type="button" class="btn primary" :disabled="oobeBusy" @click="finishOobe">
              {{ oobeBusy ? "创建中…" : oobeCreateStarted ? "重试完成" : "完成" }}
            </button>
          </div>
        </template>
      </div>
    </div>

    <template v-else>
      <div class="settings-body">
        <aside class="nav">
          <div class="nav-brand">
            <BrandMark :size="22" class="brand-icon" />
          </div>
          <nav class="nav-list">
            <button
              v-for="p in PAGES"
              :key="p.id"
              type="button"
              class="nav-item"
              :class="{ active: page === p.id }"
              :aria-current="page === p.id ? 'page' : undefined"
              @click="page = p.id"
            >
              <svg
                class="nav-ico"
                width="15"
                height="15"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                stroke-width="1.7"
                stroke-linecap="round"
                stroke-linejoin="round"
                aria-hidden="true"
              >
                <g v-html="NAV_ICONS[p.id]" />
              </svg>
              {{ p.label }}
            </button>
          </nav>
          <div class="nav-foot">FloePod · {{ s.version }}</div>
        </aside>

        <main class="content">
          <Transition name="page" mode="out-in">
            <section :key="page">
              <template v-if="page === 'general'">
                <h2 class="page-title">常规</h2>
                <p class="page-desc">浮匣的整体外观与行为。</p>
                <div class="settings-card">
                  <SettingsRow label="主题" hint="跟随系统会随 Windows 深浅色自动切换">
                    <SegmentedControl :options="THEMES" :model-value="s.theme" @update:model-value="(v) => save({ theme: v as ThemeMode })" />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="开机自启" hint="以托盘常驻方式随 Windows 启动">
                    <ToggleSwitch
                      label="开机自启"
                      :model-value="s.autostart"
                      :disabled="autostartBusy"
                      @update:model-value="saveAutostart"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="退出浮匣" hint="关闭所有匣并退出程序（托盘仍可退出）">
                    <button type="button" class="btn danger" @click="quitApp">退出</button>
                  </SettingsRow>
                </div>
              </template>

              <template v-else-if="page === 'safety'">
                <h2 class="page-title">辅助功能</h2>
                <p class="page-desc">放大界面、减少干扰，并查看或恢复每一步文件操作。</p>
                <div class="settings-card safety-settings">
                  <SettingsRow label="启用辅助功能" hint="集中启用更易读、更易点按的交互方式">
                    <ToggleSwitch
                      label="启用辅助功能"
                      :model-value="s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ enabled: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="界面大小" hint="同时放大文字、按钮和点击目标">
                    <select
                      class="input compact-select"
                      :value="s.accessibility.scale"
                      :disabled="!s.accessibility.enabled"
                      aria-label="辅助功能界面大小"
                      @change="saveAccessibility({ scale: Number(($event.target as HTMLSelectElement).value) })"
                    >
                      <option :value="1">100%</option>
                      <option :value="1.25">125%</option>
                      <option :value="1.5">150%</option>
                      <option :value="2">200%</option>
                    </select>
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="高对比度" hint="使用黑底、白字和高可见焦点框">
                    <ToggleSwitch
                      label="高对比度"
                      :model-value="s.accessibility.highContrast"
                      :disabled="!s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ highContrast: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="减少透明效果" hint="关闭模糊和玻璃效果，提高文字清晰度">
                    <ToggleSwitch
                      label="减少透明效果"
                      :model-value="s.accessibility.reduceTransparency"
                      :disabled="!s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ reduceTransparency: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="减少动画" hint="关闭弹入、缩放和过渡动画">
                    <ToggleSwitch
                      label="减少动画"
                      :model-value="s.accessibility.reduceMotion"
                      :disabled="!s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ reduceMotion: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="简明语言" hint="用完整问题代替术语和仅图标提示">
                    <ToggleSwitch
                      label="简明语言"
                      :model-value="s.accessibility.simpleLanguage"
                      :disabled="!s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ simpleLanguage: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="危险操作确认" hint="移动、批量移出前始终显示将要发生的事情">
                    <ToggleSwitch
                      label="危险操作确认"
                      :model-value="s.accessibility.confirmDangerous"
                      :disabled="!s.accessibility.enabled"
                      @update:model-value="(value) => saveAccessibility({ confirmDangerous: value })"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="资源管理器“发送到 FloePod”" hint="右键文件即可复制到第一个可用匣，作为拖拽替代">
                    <ToggleSwitch
                      label="资源管理器发送到 FloePod"
                      :model-value="s.accessibility.sendToMenu"
                      @update:model-value="(value) => saveAccessibility({ sendToMenu: value })"
                    />
                  </SettingsRow>
                </div>
                <h3 class="section-title">操作时间线与一键恢复</h3>
                <SafetyCenter />
                <h3 class="section-title search-section-title">本地 OCR、全文搜索与标签</h3>
                <SearchCenter />
                <h3 class="section-title search-section-title">机构策略、审计与诊断</h3>
                <OrganizationCenter />
              </template>

              <template v-else-if="page === 'advanced'">
                <h2 class="page-title">高级设置</h2>
                <p class="page-desc">自动屏蔽，以及每个匣的规则匣、敏感匣等进阶能力。</p>

                <h3 class="section-title">自动屏蔽</h3>
                <div class="settings-card">
                  <SettingsRow label="启用自动屏蔽" hint="按下方列表匹配前台应用">
                    <ToggleSwitch
                      label="启用自动屏蔽"
                      :model-value="s.autoBlock.enabled"
                      @update:model-value="toggleAutoBlock"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <div class="block-apps">
                    <div class="block-head">
                      <span class="block-title">屏蔽应用</span>
                      <button type="button" class="btn" @click="pickBlockApp">选择程序…</button>
                    </div>
                    <p v-if="s.autoBlock.apps.length === 0" class="block-empty">
                      还没有添加应用，按可执行文件名匹配（不区分大小写）。
                    </p>
                    <ul v-else class="block-list">
                      <li v-for="(app, index) in s.autoBlock.apps" :key="blockAppKey(app)" class="block-item">
                        <span class="block-name" :title="app">{{ app }}</span>
                        <button
                          type="button"
                          class="op-btn"
                          :aria-label="`移除 ${app}`"
                          title="移除"
                          @click="removeBlockApp(index)"
                        >
                          <svg width="13" height="13" viewBox="0 0 12 12" fill="none" stroke="currentColor" stroke-width="1.4" stroke-linecap="round">
                            <path d="m3 3 6 6M9 3l-6 6" />
                          </svg>
                        </button>
                      </li>
                    </ul>
                    <div class="block-add">
                      <input
                        v-model="blockAppDraft"
                        class="input mono"
                        maxlength="260"
                        placeholder="手动输入进程名，如 game.exe"
                        aria-label="手动输入进程名"
                        @keydown.enter.prevent="addBlockApp(blockAppDraft)"
                      />
                      <button
                        type="button"
                        class="btn"
                        :disabled="!blockAppDraft.trim()"
                        @click="addBlockApp(blockAppDraft)"
                      >
                        添加
                      </button>
                    </div>
                  </div>
                </div>

                <h3 class="section-title adv-pod-section">规则匣与敏感匣</h3>
                <p class="page-desc">按匣生效；在「匣」页可以新建、重命名或删除匣。</p>
                <div v-if="s.pods.length > 1" class="pod-picker" role="tablist" aria-label="选择要设置的匣">
                  <button
                    v-for="pod in s.pods"
                    :key="pod.id"
                    type="button"
                    role="tab"
                    class="pod-chip"
                    :class="{ active: selectedPod?.id === pod.id, off: !pod.enabled }"
                    :aria-selected="selectedPod?.id === pod.id"
                    @click="selectPod(pod.id)"
                  >
                    {{ pod.name }}
                    <span v-if="!pod.enabled" class="chip-badge">已停用</span>
                  </button>
                </div>
                <div v-for="pod in selectedPodList" :key="pod.id" class="pod-card" :class="{ off: !pod.enabled }">
                  <div class="adv-pod-head">
                    <span class="pod-name-text" :title="pod.name">{{ pod.name }}</span>
                    <span v-if="!pod.enabled" class="adv-pod-off">已停用，启用后以下设置才会生效</span>
                  </div>
                  <div class="pod-groups">
                    <div class="pod-group rules-group">
                      <div class="group-title">规则匣</div>
                      <p class="group-hint">按文件类型、名称、来源和大小过滤，并自动命名、归档或生成校验值。</p>
                      <PodRulesEditor
                        :rules="pod.rules"
                        @update="(rules) => savePod(pod.id, { rules })"
                      />
                    </div>
                    <div class="pod-group rules-group">
                      <div class="group-title">敏感匣、自动锁定与保留期限</div>
                      <p class="group-hint">使用 Windows EFS 与 Windows Hello；不上传内容，也不保存自制密码。</p>
                      <PodSecurityEditor
                        :pod-id="pod.id"
                        :folder="pod.stagingFolder"
                        :security="pod.security"
                        @update="(security) => savePod(pod.id, { security })"
                      />
                    </div>
                  </div>
                </div>
              </template>

              <template v-else-if="page === 'pods'">
                <div class="page-head">
                  <div>
                    <h2 class="page-title">匣</h2>
                    <p class="page-desc">示意图上的蓝点可直接拖动到任意边缘定位。</p>
                  </div>
                  <button type="button" class="btn" :disabled="addPodCreating" @click="openAddDialog">
                    + 新建匣
                  </button>
                </div>

                <div v-if="s.pods.length > 1" class="pod-picker" role="tablist" aria-label="选择要设置的匣">
                  <button
                    v-for="pod in s.pods"
                    :key="pod.id"
                    type="button"
                    role="tab"
                    class="pod-chip"
                    :class="{ active: selectedPod?.id === pod.id, off: !pod.enabled }"
                    :aria-selected="selectedPod?.id === pod.id"
                    @click="selectPod(pod.id)"
                  >
                    {{ pod.name }}
                    <span v-if="!pod.enabled" class="chip-badge">已停用</span>
                  </button>
                </div>

                <div v-for="pod in selectedPodList" :key="pod.id" class="pod-card" :class="{ off: !pod.enabled }">
                  <div class="pod-head">
                    <template v-if="renamingPodId === pod.id">
                      <input
                        ref="renameInput"
                        v-model="renameDraft"
                        class="pod-name-input"
                        maxlength="12"
                        aria-label="匣名称"
                        @keydown.enter.prevent="commitRename"
                        @keydown.esc.prevent="cancelRename"
                        @blur="commitRename"
                      />
                    </template>
                    <template v-else>
                      <span class="pod-name-text" :title="pod.name">{{ pod.name }}</span>
                      <button
                        type="button"
                        class="op-btn"
                        title="重命名"
                        aria-label="重命名"
                        @click="startRename(pod)"
                      >
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z" />
                          <path d="m15 5 4 4" />
                        </svg>
                      </button>
                    </template>
                    <div class="pod-head-ops">
                      <ToggleSwitch
                        :label="`启用匣 ${pod.name}`"
                        :model-value="pod.enabled"
                        :disabled="podEnabledBusyIds.has(pod.id)"
                        @update:model-value="(v) => savePodEnabled(pod, v)"
                      />
                      <button
                        type="button"
                        class="op-btn danger"
                        title="删除此匣"
                        aria-label="删除此匣"
                        :disabled="deletingPodIds.has(pod.id)"
                        @click="requestRemovePod(pod)"
                      >
                        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round">
                          <path d="M5 7h14M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m3 0-1 13a1.5 1.5 0 0 1-1.5 1.4h-7A1.5 1.5 0 0 1 6.5 20L5.5 7" />
                        </svg>
                      </button>
                    </div>
                  </div>

                  <div class="pod-groups">
                    <div class="pod-group">
                      <div class="group-title">位置</div>
                      <PodEdgeDiagram
                        :edge="pod.edge"
                        :offset="podNumberValue(pod, 'offset')"
                        :monitor-label="monitorLabel(pod)"
                        @update:edge="(e) => savePod(pod.id, { edge: e as Edge })"
                        @update:offset="(v) => previewPodNumber(pod.id, 'offset', v)"
                        @commit="(v) => commitPodNumber(pod, 'offset', v)"
                      />
                      <div class="frow">
                        <span class="flabel">屏幕边缘</span>
                        <div class="fctrl">
                          <SegmentedControl :options="EDGES" :model-value="pod.edge" @update:model-value="(v) => savePod(pod.id, { edge: v as Edge })" />
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">显示器</span>
                        <div class="fctrl">
                          <select
                            :value="pod.monitor"
                            class="input sel"
                            @change="(e) => commitPodMonitor(pod, e)"
                          >
                            <option value="">主显示器</option>
                            <option v-for="m in monitors" :key="m.name" :value="m.name">{{ m.label }}</option>
                          </select>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">沿边缘位置</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'offset')"
                            :min="0"
                            :max="1"
                            :step="0.01"
                            aria-label="沿边缘位置"
                            @update:value="(v) => previewPodNumber(pod.id, 'offset', v)"
                            @commit="(v) => commitPodNumber(pod, 'offset', v)"
                          />
                          <span class="fval">{{ Math.round(podNumberValue(pod, "offset") * 100) }}%</span>
                        </div>
                      </div>
                    </div>

                    <div class="pod-group">
                      <div class="group-title">边缘浮动条</div>
                      <div class="frow">
                        <span class="flabel">匣宽度</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'barWidth')"
                            :min="28"
                            :max="96"
                            :step="2"
                            aria-label="匣宽度"
                            @update:value="(v) => previewPodNumber(pod.id, 'barWidth', v)"
                            @commit="(v) => commitPodNumber(pod, 'barWidth', v)"
                          />
                          <span class="fval">{{ podNumberValue(pod, "barWidth") }}px</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">圆角</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'cornerRadius')"
                            :min="0"
                            :max="32"
                            :step="1"
                            aria-label="圆角"
                            @update:value="(v) => previewPodNumber(pod.id, 'cornerRadius', v)"
                            @commit="(v) => commitPodNumber(pod, 'cornerRadius', v)"
                          />
                          <span class="fval">{{ podNumberValue(pod, "cornerRadius") }}px</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">不透明度</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'opacity')"
                            :min="0.1"
                            :max="1"
                            :step="0.01"
                            aria-label="不透明度"
                            @update:value="(v) => previewPodNumber(pod.id, 'opacity', v)"
                            @commit="(v) => commitPodNumber(pod, 'opacity', v)"
                          />
                          <span class="fval">{{ Math.round(podNumberValue(pod, "opacity") * 100) }}%</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">边框颜色</span>
                        <div class="fctrl">
                          <label class="color-field">
                            <input
                              type="color"
                              class="color-input"
                              :value="podHexColorValue(colorDraftValue(pod, 'borderColor'))"
                              aria-label="边框颜色"
                              @input="(e) => previewPodColor(pod, 'borderColor', (e.target as HTMLInputElement).value)"
                            />
                            <span class="color-text">{{ colorDraftValue(pod, 'borderColor') || "跟随主题" }}</span>
                          </label>
                          <button
                            v-if="hasColorDraft(pod, 'borderColor')"
                            type="button"
                            class="btn primary"
                            @click="confirmPodColor(pod, 'borderColor')"
                          >
                            确定
                          </button>
                          <button
                            v-if="pod.borderColor"
                            type="button"
                            class="btn ghost"
                            @click="clearPodColor(pod, 'borderColor')"
                          >
                            重置
                          </button>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">边框不透明度</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'borderOpacity')"
                            :min="0"
                            :max="1"
                            :step="0.01"
                            aria-label="边框不透明度"
                            @update:value="(v) => previewPodNumber(pod.id, 'borderOpacity', v)"
                            @commit="(v) => commitPodNumber(pod, 'borderOpacity', v)"
                          />
                          <span class="fval">{{ Math.round(podNumberValue(pod, "borderOpacity") * 100) }}%</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">隐匿模式</span>
                        <div class="fctrl">
                          <ToggleSwitch
                            label="隐匿模式"
                            :model-value="pod.stealth"
                            @update:model-value="(v) => savePod(pod.id, { stealth: v })"
                          />
                        </div>
                      </div>
                      <div v-if="pod.stealth" class="frow">
                        <span class="flabel">隐匿延迟</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'stealthDelayMs')"
                            :min="500"
                            :max="20000"
                            :step="250"
                            aria-label="隐匿延迟"
                            @update:value="(v) => previewPodNumber(pod.id, 'stealthDelayMs', v)"
                            @commit="(v) => commitPodNumber(pod, 'stealthDelayMs', v)"
                          />
                          <span class="fval">{{ (podNumberValue(pod, "stealthDelayMs") / 1000).toFixed(2).replace(/\.?0+$/, "") }}s</span>
                        </div>
                      </div>
                    </div>

                    <div class="pod-group">
                      <div class="group-title">浮动面板</div>
                      <div class="frow">
                        <span class="flabel">浮动面板宽度</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'panelWidth')"
                            :min="300"
                            :max="520"
                            :step="10"
                            aria-label="浮动面板宽度"
                            @update:value="(v) => previewPodNumber(pod.id, 'panelWidth', v)"
                            @commit="(v) => commitPodNumber(pod, 'panelWidth', v)"
                          />
                          <span class="fval">{{ podNumberValue(pod, "panelWidth") }}px</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">浮动面板材质</span>
                        <div class="fctrl">
                          <SegmentedControl :options="MATERIALS" :model-value="pod.panelMaterial" @update:model-value="(v) => savePod(pod.id, { panelMaterial: v as Material })" />
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">浮动面板填充色</span>
                        <div class="fctrl">
                          <label class="color-field">
                            <input
                              type="color"
                              class="color-input"
                              :value="podHexColorValue(colorDraftValue(pod, 'panelColor'))"
                              aria-label="浮动面板填充色"
                              @input="(e) => previewPodColor(pod, 'panelColor', (e.target as HTMLInputElement).value)"
                            />
                            <span class="color-text">{{ colorDraftValue(pod, 'panelColor') || "跟随主题" }}</span>
                          </label>
                          <button
                            v-if="hasColorDraft(pod, 'panelColor')"
                            type="button"
                            class="btn primary"
                            @click="confirmPodColor(pod, 'panelColor')"
                          >
                            确定
                          </button>
                          <button
                            v-if="pod.panelColor"
                            type="button"
                            class="btn ghost"
                            @click="clearPodColor(pod, 'panelColor')"
                          >
                            重置
                          </button>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">浮动面板填充色不透明度</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'panelOpacity')"
                            :min="0.1"
                            :max="1"
                            :step="0.01"
                            aria-label="浮动面板填充色不透明度"
                            @update:value="(v) => previewPodNumber(pod.id, 'panelOpacity', v)"
                            @commit="(v) => commitPodNumber(pod, 'panelOpacity', v)"
                          />
                          <span class="fval">{{ Math.round(podNumberValue(pod, "panelOpacity") * 100) }}%</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">悬停自动打开</span>
                        <div class="fctrl">
                          <ToggleSwitch
                            label="悬停自动打开"
                            :model-value="pod.hoverOpen"
                            @update:model-value="(v) => savePod(pod.id, { hoverOpen: v })"
                          />
                        </div>
                      </div>
                      <div v-if="pod.hoverOpen" class="frow">
                        <span class="flabel">悬停展开延迟</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'hoverDelayMs')"
                            :min="0"
                            :max="400"
                            :step="20"
                            aria-label="悬停展开延迟"
                            @update:value="(v) => previewPodNumber(pod.id, 'hoverDelayMs', v)"
                            @commit="(v) => commitPodNumber(pod, 'hoverDelayMs', v)"
                          />
                          <span class="fval">{{ podNumberValue(pod, "hoverDelayMs") }}ms</span>
                        </div>
                      </div>
                      <div class="frow">
                        <span class="flabel">浮动面板自动收起</span>
                        <div class="fctrl">
                          <ToggleSwitch
                            label="浮动面板自动收起"
                            :model-value="pod.autoHide"
                            @update:model-value="(v) => savePod(pod.id, { autoHide: v })"
                          />
                        </div>
                      </div>
                      <div v-if="pod.autoHide" class="frow">
                        <span class="flabel">收起延迟</span>
                        <div class="fctrl">
                          <RangeSlider
                            :value="podNumberValue(pod, 'autoHideDelayMs')"
                            :min="0"
                            :max="2000"
                            :step="20"
                            aria-label="收起延迟"
                            @update:value="(v) => previewPodNumber(pod.id, 'autoHideDelayMs', v)"
                            @commit="(v) => commitPodNumber(pod, 'autoHideDelayMs', v)"
                          />
                          <span class="fval">{{ podNumberValue(pod, "autoHideDelayMs") }}ms</span>
                        </div>
                      </div>
                    </div>

                    <div class="pod-group">
                      <div class="group-title">拖入</div>
                      <div class="frow">
                        <span class="flabel">落地动作</span>
                        <div class="fctrl">
                          <SegmentedControl :options="DROP_ACTIONS" :model-value="pod.dropAction" @update:model-value="(v) => savePod(pod.id, { dropAction: v as DropAction })" />
                        </div>
                      </div>
                      <div class="frow folder-row">
                        <span class="flabel">暂存文件夹</span>
                        <div class="fctrl folder-line">
                          <input :value="pod.stagingFolder" class="input mono" readonly :title="pod.stagingFolder" placeholder="未选择" />
                          <button type="button" class="btn" @click="changePodFolder(pod)">选择…</button>
                          <button v-if="pod.stagingFolder" type="button" class="btn ghost" @click="openPodFolder(pod)">打开</button>
                        </div>
                      </div>
                    </div>
                  </div>
                </div>
              </template>

              <template v-else-if="page === 'hotkeys'">
                <h2 class="page-title">快捷键</h2>
                <p class="page-desc">全局快捷键，点击后按下新组合即可修改。</p>
                <div class="settings-card">
                  <SettingsRow label="显示 / 隐藏全部匣">
                    <HotkeyRecorder :model-value="s.hotkeys.toggleBar" @update:model-value="(v) => saveHotkey('toggleBar', v)" />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="收集剪贴板文字" hint="把当前剪贴板里的文字存为第一匣的暂存">
                    <HotkeyRecorder :model-value="s.hotkeys.collectClipboard" @update:model-value="(v) => saveHotkey('collectClipboard', v)" />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="打开第一匣浮动面板">
                    <HotkeyRecorder :model-value="s.hotkeys.openPanel" @update:model-value="(v) => saveHotkey('openPanel', v)" />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="紧急锁定敏感匣" hint="立即清除所有内存中的敏感匣解锁状态">
                    <HotkeyRecorder :model-value="s.hotkeys.lockSensitive" @update:model-value="(v) => saveHotkey('lockSensitive', v)" />
                  </SettingsRow>
                </div>
                <p v-if="hotkeyError" class="error">{{ hotkeyError }}</p>
                <div class="reset-line">
                  <button type="button" class="btn ghost" @click="resetHotkeys">恢复默认快捷键</button>
                </div>
              </template>

              <template v-else>
                <div class="about-hero">
                  <BrandMark :size="48" class="about-brand" />
                  <div class="about-ver">版本 {{ s.version }}</div>
                </div>
                <p class="about-text">
                  本地优先的屏幕边缘暂存工具：拖进来集中保管，拖出去继续使用。<br />
                  不联网、不收集数据，所有内容只存在你自己的电脑上。
                </p>
                <div class="about-meta">
                  <div class="about-row">
                    <span class="about-key">数据位置</span>
                    <span class="about-val">{{ s.dataDir }}</span>
                  </div>
                  <div class="about-row">
                    <span class="about-key">匣的数量</span>
                    <span class="about-val">{{ s.pods.length }} 个</span>
                  </div>
                </div>
                <div class="reset-line about-update-line">
                  <button type="button" class="btn" @click="checkUpdates">查看最新版本与下载</button>
                </div>
              </template>
            </section>
          </Transition>
        </main>
      </div>
    </template>
    </template>

    <!-- 删除匣：选择仅移除还是连暂存文件夹一起删除 -->
    <Transition name="modal">
      <div v-if="deleteDialog" class="modal-layer" @pointerdown.self="cancelRemovePod">
        <div class="modal-card" role="dialog" aria-modal="true" aria-label="删除匣">
          <h3 class="modal-title">删除「{{ deleteDialog.pod.name }}」</h3>
          <p class="modal-text">暂存文件夹：{{ deleteDialog.pod.stagingFolder || "未设置" }}</p>
          <div class="modal-actions column">
            <button type="button" class="btn" @click="removePodKeepingFiles">
              仅移除匣（保留文件夹和文件）
            </button>
            <button type="button" class="btn danger" @click="removePodWithFolder">
              删除匣，并把暂存文件夹与文件移入回收站
            </button>
            <button type="button" class="btn ghost" @click="cancelRemovePod">取消</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 新建匣：先询问名称与文件夹位置 -->
    <Transition name="modal">
      <div v-if="addDialogOpen" class="modal-layer" @pointerdown.self="cancelAddDialog">
        <div class="modal-card" role="dialog" aria-modal="true" aria-label="新建匣">
          <h3 class="modal-title">新建匣</h3>
          <label class="field">
            <span>名称</span>
            <input v-model="addDraft.name" class="input" maxlength="12" placeholder="匣名称" />
          </label>
          <div class="field">
            <span>保存文件夹</span>
            <div class="folder-line">
              <input :value="addDraft.folder" class="input mono" readonly placeholder="选择存放暂存文件的文件夹" />
              <button type="button" class="btn" :disabled="addPodCreating" @click="chooseAddFolder">
                选择…
              </button>
            </div>
          </div>
          <div class="modal-actions">
            <button type="button" class="btn ghost" :disabled="addPodCreating" @click="cancelAddDialog">
              取消
            </button>
            <button
              type="button"
              class="btn primary"
              :disabled="addPodCreating || !addDraft.folder"
              @click="confirmAddPod"
            >
              {{ addPodCreating ? "创建中…" : "创建" }}
            </button>
          </div>
        </div>
      </div>
    </Transition>

    <Transition name="toast">
      <div v-if="toast" class="toast">{{ toast }}</div>
    </Transition>
  </div>
</template>

<style scoped>
.settings-root {
  position: fixed;
  inset: 0;
  display: flex;
  flex-direction: column;
  background: var(--surface);
  color: var(--ink);
}
.load-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  color: var(--ink-2);
  font-size: 13px;
  text-align: center;
  padding: 32px;
}
.load-state p {
  margin: 0;
}
.load-state.error-state {
  color: var(--danger);
}

.titlebar {
  flex-shrink: 0;
  height: 34px;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding-left: 14px;
  background: var(--surface-raised);
  border-bottom: 1px solid var(--line);
  user-select: none;
}
.titlebar-title {
  font-size: 12px;
  color: var(--ink-2);
  letter-spacing: 0.02em;
}
.titlebar-controls {
  display: flex;
  height: 100%;
}
.tb-btn {
  width: 42px;
  height: 100%;
  border: 0;
  background: transparent;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--ink-2);
  cursor: pointer;
  font-family: inherit;
  transition: background 150ms var(--ease-out), color 150ms var(--ease-out);
}
.tb-btn:hover {
  background: var(--surface-hover);
  color: var(--ink);
}
.tb-btn.close:hover {
  background: var(--danger);
  color: var(--on-danger);
}

.oobe {
  flex: 1;
  min-height: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--surface);
  overflow: auto;
}
.oobe-card {
  width: 380px;
  max-width: calc(100% - 48px);
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 16px;
  text-align: center;
  background: var(--surface-raised);
  border-radius: 18px;
  box-shadow: var(--shadow-panel), 0 0 0 1px var(--line);
  padding: 34px 30px;
}
.oobe-brand {
  color: var(--accent);
}
.oobe-title {
  margin: 0;
  font-size: 21px;
  font-weight: 680;
  letter-spacing: -0.015em;
}
.oobe-step-title {
  margin: 0;
  font-size: 17px;
  font-weight: 650;
}
.oobe-text {
  margin: 0;
  font-size: 13px;
  line-height: 1.75;
  color: var(--ink-2);
}
.oobe-text.dim {
  color: var(--ink-3);
  font-size: 12.5px;
}
.oobe-form {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 14px;
  text-align: left;
}
.field {
  display: flex;
  flex-direction: column;
  gap: 6px;
  font-size: 12.5px;
  font-weight: 550;
  color: var(--ink-2);
}
.oobe-actions {
  display: flex;
  gap: 10px;
  justify-content: center;
  margin-top: 6px;
}

.settings-body {
  flex: 1;
  display: flex;
  min-height: 0;
}
.nav {
  width: 172px;
  flex-shrink: 0;
  background: var(--surface-raised);
  display: flex;
  flex-direction: column;
  padding: 18px 10px 14px;
  border-right: 1px solid var(--line);
}
.nav-brand {
  display: flex;
  align-items: center;
  gap: 9px;
  font-size: 15px;
  font-weight: 650;
  letter-spacing: -0.01em;
  padding: 0 10px 18px;
  border-radius: 8px;
  cursor: default;
  transition: transform 160ms var(--ease-out), filter 160ms var(--ease-out);
}
/* Logo 反馈：悬停轻微放大提亮，按下回缩 */
.nav-brand:hover {
  transform: scale(1.06);
  filter: brightness(1.12);
}
.nav-brand:active {
  transform: scale(0.94);
  filter: brightness(0.96);
  transition-duration: 80ms;
}
.brand-icon {
  color: var(--accent);
}
.nav-list {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.nav-item {
  position: relative;
  display: flex;
  align-items: center;
  gap: 10px;
  border: 0;
  background: transparent;
  width: 100%;
  text-align: left;
  padding: 8px 12px;
  border-radius: 8px;
  font-size: 13px;
  color: var(--ink-2);
  cursor: pointer;
  font-family: inherit;
  transition: background 160ms var(--ease-out), color 160ms var(--ease-out);
}
.nav-ico {
  flex-shrink: 0;
  opacity: 0.85;
}
.nav-item:hover {
  background: var(--surface-hover);
  color: var(--ink);
}
.nav-item.active {
  background: var(--accent-soft);
  color: var(--accent);
  font-weight: 600;
}
.nav-item.active .nav-ico {
  opacity: 1;
}
.nav-foot {
  margin-top: auto;
  padding: 0 10px;
  font-size: 11px;
  color: var(--ink-3);
}

.content {
  flex: 1;
  min-width: 0;
  overflow-y: auto;
  padding: 26px 30px 34px;
}
.page-title {
  font-size: 19px;
  font-weight: 650;
  letter-spacing: -0.015em;
  margin: 0 0 4px;
}
.page-head {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 12px;
}
/* 标题旁的操作按钮不参与伸缩，避免被长描述挤变形 */
.page-head .btn {
  flex-shrink: 0;
  white-space: nowrap;
}
.page-desc {
  font-size: 12.5px;
  color: var(--ink-3);
  line-height: 1.6;
  margin: 0 0 14px;
}
.page-head .page-desc {
  margin-bottom: 18px;
}
.sep {
  height: 1px;
  background: var(--line);
  margin: 0 16px;
}
.settings-card {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
  overflow: hidden;
}
.settings-card :deep(.row) {
  padding: 14px 16px;
}
.safety-settings {
  margin-bottom: 20px;
}
.compact-select {
  width: 116px;
}
.section-title {
  margin: 0 0 10px;
  font-size: 14px;
  font-weight: 700;
}
.search-section-title {
  margin-top: 24px;
}

.page-enter-active,
.page-leave-active {
  transition: opacity 170ms ease, transform 240ms var(--ease-out);
}
.page-enter-from {
  opacity: 0;
  transform: translateY(8px);
}
.page-leave-to {
  opacity: 0;
  transform: translateY(-6px);
}

.btn {
  border: 1px solid var(--line-strong);
  background: var(--surface-raised);
  color: var(--ink);
  border-radius: 8px;
  padding: 6px 13px;
  font-size: 12.5px;
  font-weight: 550;
  cursor: pointer;
  font-family: inherit;
  transition: background 150ms var(--ease-out), border-color 150ms var(--ease-out),
    transform 100ms var(--ease-out);
}
.btn:active {
  transform: scale(0.97);
}
.btn:hover {
  background: var(--surface-hover);
}
.btn.primary {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--on-accent);
}
.btn.primary:hover {
  background: var(--accent-hover);
}
/* 退出等破坏性动作：常态即带红色描边，悬停整块填充 */
.btn.danger {
  color: var(--danger);
  border-color: color-mix(in oklab, var(--danger) 45%, transparent);
}
.btn.danger:hover {
  background: var(--danger);
  border-color: var(--danger);
  color: var(--on-danger);
}
.btn.ghost {
  border-color: transparent;
  color: var(--ink-2);
}
.btn.ghost:hover {
  border-color: var(--line-strong);
}
.btn:disabled {
  opacity: 0.45;
  cursor: default;
  pointer-events: none;
}
.input {
  border: 1px solid var(--line-strong);
  border-radius: 8px;
  padding: 6px 10px;
  font-size: 12.5px;
  background: var(--surface-raised);
  color: var(--ink);
  outline: none;
  font-family: inherit;
}
.input:focus {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-soft);
}
.input.mono {
  flex: 1;
  min-width: 0;
  font-size: 11.5px;
  color: var(--ink-2);
}
select.input {
  cursor: pointer;
}
.input.sel {
  min-width: 140px;
}
.slider-line {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
}
.folder-line {
  display: flex;
  align-items: center;
  gap: 8px;
  width: 100%;
}
.error {
  font-size: 12px;
  color: var(--danger);
  margin: 10px 0 0;
}
.reset-line {
  margin-top: 18px;
}
/* 关于页的版本按钮与居中的 about-hero / about-meta 对齐 */
.about-update-line {
  display: flex;
  justify-content: center;
}

.pod-picker {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 16px;
}
.pod-chip {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  border: 1px solid var(--line-strong);
  background: var(--surface-raised);
  color: var(--ink-2);
  border-radius: 999px;
  padding: 6px 14px;
  font-size: 12.5px;
  font-weight: 550;
  cursor: pointer;
  font-family: inherit;
  transition: background 150ms var(--ease-out), color 150ms var(--ease-out),
    border-color 150ms var(--ease-out);
}
.pod-chip:hover {
  background: var(--surface-hover);
  color: var(--ink);
}
.pod-chip.active {
  background: var(--accent-soft);
  border-color: var(--accent);
  color: var(--accent);
  font-weight: 600;
}
.pod-chip.off {
  opacity: 0.55;
}
.chip-badge {
  font-size: 10px;
  color: var(--ink-3);
}
.pod-card {
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
  padding: 16px 18px;
  box-shadow: 0 8px 24px -22px rgb(0 0 0 / 0.55);
}
.pod-card.off {
  opacity: 0.6;
}
.pod-head {
  display: flex;
  align-items: center;
  gap: 6px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--line);
  margin-bottom: 12px;
}
.pod-name-text {
  font-size: 15px;
  font-weight: 650;
  color: var(--ink);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 320px;
  padding: 2px 4px;
}
.pod-name-input {
  border: 0;
  background: transparent;
  font-size: 15px;
  font-weight: 650;
  color: var(--ink);
  outline: none;
  font-family: inherit;
  padding: 2px 4px;
  border-radius: 6px;
}
.pod-name-input:focus {
  background: var(--surface-hover);
}
.pod-head-ops {
  margin-left: auto;
  display: flex;
  align-items: center;
  gap: 10px;
}
.op-btn {
  border: 0;
  background: transparent;
  color: var(--ink-3);
  width: 28px;
  height: 28px;
  border-radius: 8px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: pointer;
  font-family: inherit;
  transition: background 150ms var(--ease-out), color 150ms var(--ease-out);
}
.op-btn:hover {
  background: var(--surface-hover);
  color: var(--ink);
}
.op-btn:disabled {
  cursor: wait;
  opacity: 0.5;
}
.op-btn.danger:hover {
  background: color-mix(in oklab, var(--danger) 14%, transparent);
  color: var(--danger);
}

/* 分组字段：标签左对齐，控件右对齐，分区标题建立扫视结构 */
.pod-groups {
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.group-title {
  font-size: 11px;
  font-weight: 650;
  letter-spacing: 0.05em;
  color: var(--ink-3);
  margin: 10px 0 2px;
}
.pod-group:first-child .group-title {
  margin-top: 0;
}
.group-hint {
  margin: 0 0 8px;
  color: var(--ink-3);
  font-size: 11.5px;
}
.rules-group {
  margin-top: 6px;
}
/* 高级设置页:按匣展示规则匣与敏感匣 */
.adv-pod-section {
  margin-top: 26px;
}
.adv-pod-head {
  display: flex;
  align-items: baseline;
  gap: 10px;
  padding-bottom: 12px;
  border-bottom: 1px solid var(--line);
  margin-bottom: 12px;
}
.adv-pod-off {
  font-size: 11.5px;
  color: var(--ink-3);
}
.frow {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 20px;
  padding: 5px 0;
}
.flabel {
  font-size: 13px;
  color: var(--ink-2);
  white-space: nowrap;
}
.fctrl {
  display: flex;
  align-items: center;
  gap: 10px;
  min-width: 0;
}
.fval {
  min-width: 40px;
  text-align: right;
  font-size: 12px;
  color: var(--ink-3);
  font-variant-numeric: tabular-nums;
}
.folder-row .folder-line {
  max-width: 460px;
}

/* 新建匣弹窗 */
.modal-layer {
  position: fixed;
  inset: 0;
  z-index: 60;
  display: flex;
  align-items: center;
  justify-content: center;
  background: oklch(0 0 0 / 0.35);
}
.modal-card {
  width: 380px;
  max-width: calc(100% - 48px);
  display: flex;
  flex-direction: column;
  gap: 14px;
  background: var(--surface-raised);
  border-radius: 14px;
  box-shadow: var(--shadow-panel), 0 0 0 1px var(--line);
  padding: 20px 22px 18px;
}
.modal-title {
  margin: 0;
  font-size: 16px;
  font-weight: 650;
}
.modal-text {
  margin: 0;
  font-size: 12px;
  line-height: 1.6;
  color: var(--ink-2);
  overflow-wrap: anywhere;
}
.modal-actions {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  margin-top: 2px;
}
.modal-actions.column {
  flex-direction: column;
  align-items: stretch;
}
.modal-actions.column .btn {
  text-align: center;
}
.modal-enter-active,
.modal-leave-active {
  transition: opacity 160ms ease;
}
.modal-enter-active .modal-card,
.modal-leave-active .modal-card {
  transition: transform 200ms var(--ease-out), opacity 160ms ease;
}
.modal-enter-from,
.modal-leave-to {
  opacity: 0;
}
.modal-enter-from .modal-card,
.modal-leave-to .modal-card {
  transform: translateY(10px) scale(0.98);
  opacity: 0;
}

.color-field {
  display: inline-flex;
  align-items: center;
  gap: 8px;
}
.color-input {
  width: 32px;
  height: 24px;
  padding: 0;
  border: 1px solid var(--line-strong);
  border-radius: 6px;
  background: var(--surface-raised);
  cursor: pointer;
}
.color-input::-webkit-color-swatch-wrapper {
  padding: 2px;
}
.color-input::-webkit-color-swatch {
  border: none;
  border-radius: 3px;
}
.color-text {
  font-size: 12px;
  color: var(--ink-3);
  font-variant-numeric: tabular-nums;
}

/* 自动屏蔽 */
.block-apps {
  padding: 14px 16px 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}
.block-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
}
.block-title {
  font-size: 13px;
  font-weight: 600;
  color: var(--ink);
}
.block-list {
  list-style: none;
  margin: 0;
  padding: 0;
  border: 1px solid var(--line);
  border-radius: 10px;
  overflow: hidden;
}
.block-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 10px;
  padding: 7px 8px 7px 12px;
  font-size: 12.5px;
  font-variant-numeric: tabular-nums;
}
.block-item + .block-item {
  border-top: 1px solid var(--line);
}
.block-item:hover {
  background: var(--surface-hover);
}
.block-name {
  font-family: ui-monospace, Consolas, monospace;
  color: var(--ink-2);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.block-empty {
  margin: 0;
  font-size: 12px;
  line-height: 1.7;
  color: var(--ink-3);
  background: var(--surface-hover);
  border-radius: 10px;
  padding: 12px 14px;
}
.block-add {
  display: flex;
  gap: 8px;
}
.block-add .input {
  flex: 1;
  min-width: 0;
}
.block-tip {
  margin-top: 12px;
  margin-bottom: 0;
}

.about-hero {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 6px;
  padding: 22px 0 8px;
}
.about-brand {
  color: var(--accent);
}
.about-name {
  font-size: 18px;
  font-weight: 680;
  letter-spacing: -0.015em;
}
.about-ver {
  font-size: 12px;
  color: var(--ink-3);
}
.about-text {
  font-size: 13px;
  line-height: 1.8;
  color: var(--ink-2);
  max-width: 430px;
  margin: 6px auto 20px;
  text-align: center;
}
.about-meta {
  max-width: 430px;
  margin: 0 auto;
  border: 1px solid var(--line);
  border-radius: 12px;
  background: var(--surface-raised);
  overflow: hidden;
}
.about-row {
  display: flex;
  align-items: baseline;
  gap: 16px;
  padding: 10px 14px;
  font-size: 12.5px;
}
.about-row + .about-row {
  border-top: 1px solid var(--line);
}
.about-key {
  flex-shrink: 0;
  color: var(--ink-2);
  font-weight: 550;
  width: 64px;
}
.about-val {
  color: var(--ink);
  overflow-wrap: anywhere;
}

.toast {
  position: absolute;
  bottom: 18px;
  left: 50%;
  transform: translateX(-50%);
  background: var(--ink);
  color: var(--surface);
  font-size: 12px;
  padding: 8px 16px;
  border-radius: 999px;
  box-shadow: var(--shadow-pop);
}
.toast-enter-active,
.toast-leave-active {
  transition: opacity 180ms ease, transform 240ms var(--ease-out);
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(8px);
}
</style>
