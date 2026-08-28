<script setup lang="ts">
/**
 * 设置窗口：OOBE 首启引导 / 常规 / 匣管理 / 快捷键 / 关于。
 * 所有修改即时保存（save -> Rust 持久化并广播 settings_changed）。
 * 排版原则：单一强调色、层级靠字号与留白、分区标题建立可扫视结构、
 * 进入/切换动画统一使用出程缓动（--ease-out）。
 */
import { computed, onBeforeUnmount, onMounted, reactive, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { useToast } from "@/composables/useToast";
import { normalizeWindowsPathKey } from "@/domain/settings";
import type { DropAction, Edge, Material, Pod, ThemeMode } from "@/domain/types";
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

const settingsStore = useSettingsStore();
const s = computed(() => settingsStore.settings);
const monitors = computed(() => settingsStore.monitors);

const page = ref<"general" | "pods" | "hotkeys" | "about">("general");
const { toast, showToast, disposeToast } = useToast(2400);
const hotkeyError = ref("");
const loading = ref(true);
const loadError = ref("");
const addPodBusy = ref(false);
const autostartBusy = ref(false);
const deletingPodIds = reactive(new Set<number>());
const podEnabledBusyIds = reactive(new Set<number>());
let settingsSaveTail: Promise<void> = Promise.resolve();
const podSaveTails = new Map<number, Promise<void>>();
let hotkeySaveRevision = 0;

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
  { value: "right", label: "右" },
  { value: "bottom", label: "下" },
  { value: "left", label: "左" },
];

const DROP_ACTIONS: { value: DropAction; label: string }[] = [
  { value: "ask", label: "询问" },
  { value: "copy", label: "复制" },
  { value: "move", label: "移动" },
  { value: "shortcut", label: "快捷方式" },
];

const MATERIALS: { value: Material; label: string }[] = [
  { value: "acrylic", label: "亚克力" },
  { value: "plain", label: "纯半透明" },
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

type PodNumberField = "offset" | "opacity" | "panelWidth" | "hoverDelayMs";
const podNumberDrafts = reactive<Record<number, Partial<Record<PodNumberField, number>>>>({});
const podNameDrafts = reactive<Record<number, string>>({});

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

function previewPodName(id: number, event: Event) {
  podNameDrafts[id] = (event.target as HTMLInputElement).value;
}

async function commitPodName(pod: Pod, event: Event) {
  previewPodName(pod.id, event);
  const value = podNameDrafts[pod.id];
  if (value == null) return;
  if (value !== pod.name) await savePod(pod.id, { name: value });
  // 清空受控草稿后，保存失败时也能恢复已持久化的值。
  if (podNameDrafts[pod.id] === value) delete podNameDrafts[pod.id];
}

async function commitPodMonitor(pod: Pod, event: Event) {
  const select = event.target as HTMLSelectElement;
  const value = select.value;
  const saved = await savePod(pod.id, { monitor: value });
  if (!saved && select.value === value) {
    select.value = settingsStore.pod(pod.id)?.monitor ?? pod.monitor;
  }
}

async function addPod() {
  if (addPodBusy.value) return;
  addPodBusy.value = true;
  try {
    const folder = await pickFolder();
    if (!folder) return;
    const n = s.value?.pods.length ?? 0;
    const edge = (["left", "right", "top", "bottom"] as Edge[])[n % 4];
    await ipc.createPod({
      name: `匣 ${n + 1}`,
      edge,
      monitor: "",
      offset: 0.5,
      stagingFolder: folder,
      opacity: 1,
      material: "acrylic",
      panelWidth: 380,
      hoverDelayMs: 120,
      dropAction: "ask",
      enabled: true,
    });
    await settingsStore.refreshPods();
    showToast("已创建新匣");
  } catch (err) {
    console.error(err);
    showToast("创建失败，请重试");
  } finally {
    addPodBusy.value = false;
  }
}

async function removePod(pod: Pod) {
  if (deletingPodIds.has(pod.id)) return;
  deletingPodIds.add(pod.id);
  try {
    const message = `删除「${pod.name}」？其中的暂存文件会一并移入回收站。`;
    const ok = ipc.inTauri
      ? await ask(message, { title: "删除匣", kind: "warning" })
      : window.confirm(message);
    if (!ok) return;
    await ipc.deletePod(pod.id, true);
    await settingsStore.refreshPods();
    showToast("已删除");
  } catch (err) {
    console.error(err);
    showToast("删除失败，请重试");
  } finally {
    deletingPodIds.delete(pod.id);
  }
}

function oobePodConfig(): Omit<Pod, "id"> {
  return {
    name: oobe.value.name || "我的匣",
    edge: oobe.value.edge,
    monitor: oobe.value.monitor,
    offset: 0.5,
    stagingFolder: oobe.value.folder,
    opacity: Number(oobe.value.opacity),
    material: oobe.value.material,
    panelWidth: 380,
    hoverDelayMs: 120,
    dropAction: "ask",
    enabled: true,
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

async function saveHotkey(key: "toggleBar" | "collectClipboard" | "openPanel", combo: string) {
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
          material: existing.material,
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
  { id: "hotkeys", label: "快捷键" },
  { id: "about", label: "关于" },
] as const;
</script>

<template>
  <div class="settings-root">
    <div class="titlebar" data-tauri-drag-region>
      <div class="titlebar-title" data-tauri-drag-region>浮匣 设置</div>
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
              <span>材质</span>
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
                      :model-value="s.autostart"
                      :disabled="autostartBusy"
                      @update:model-value="saveAutostart"
                    />
                  </SettingsRow>
                  <div class="sep" />
                  <SettingsRow label="退出浮匣" hint="关闭所有匣并退出程序（托盘仍可退出）">
                    <button type="button" class="btn" @click="quitApp">退出</button>
                  </SettingsRow>
                </div>
              </template>

              <template v-else-if="page === 'pods'">
                <div class="page-head">
                  <div>
                    <h2 class="page-title">匣</h2>
                    <p class="page-desc">每个匣是贴在屏幕边缘的独立暂存点，可分别设置位置、显示器和保存文件夹。</p>
                  </div>
                  <button type="button" class="btn" :disabled="addPodBusy" @click="addPod">
                    {{ addPodBusy ? "创建中…" : "+ 新建匣" }}
                  </button>
                </div>

                <TransitionGroup name="pod" tag="div" class="pod-list">
                  <div v-for="pod in s.pods" :key="pod.id" class="pod-card" :class="{ off: !pod.enabled }">
                    <div class="pod-head">
                      <input
                        :value="podNameDrafts[pod.id] ?? pod.name"
                        class="pod-name-input"
                        maxlength="12"
                        @input="(e) => previewPodName(pod.id, e)"
                        @change="(e) => commitPodName(pod, e)"
                      />
                      <div class="pod-head-ops">
                        <ToggleSwitch
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
                          @click="removePod(pod)"
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
                        <div class="group-title">外观</div>
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
                          <span class="flabel">材质</span>
                          <div class="fctrl">
                            <SegmentedControl :options="MATERIALS" :model-value="pod.material" @update:model-value="(v) => savePod(pod.id, { material: v as Material })" />
                          </div>
                        </div>
                      </div>

                      <div class="pod-group">
                        <div class="group-title">面板</div>
                        <div class="frow">
                          <span class="flabel">面板宽度</span>
                          <div class="fctrl">
                            <RangeSlider
                              :value="podNumberValue(pod, 'panelWidth')"
                              :min="300"
                              :max="520"
                              :step="10"
                              aria-label="面板宽度"
                              @update:value="(v) => previewPodNumber(pod.id, 'panelWidth', v)"
                              @commit="(v) => commitPodNumber(pod, 'panelWidth', v)"
                            />
                            <span class="fval">{{ podNumberValue(pod, "panelWidth") }}px</span>
                          </div>
                        </div>
                        <div class="frow">
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
                </TransitionGroup>
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
                  <SettingsRow label="打开第一匣面板">
                    <HotkeyRecorder :model-value="s.hotkeys.openPanel" @update:model-value="(v) => saveHotkey('openPanel', v)" />
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
                  本地优先的屏幕边缘暂存工具：拖进来集中保管，拖出去继续使用。
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
              </template>
            </section>
          </Transition>
        </main>
      </div>
    </template>
    </template>

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
  gap: 20px;
}
.page-desc {
  font-size: 12.5px;
  color: var(--ink-3);
  line-height: 1.65;
  margin: 0 0 18px;
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

.pod-list {
  display: flex;
  flex-direction: column;
  gap: 14px;
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
  gap: 10px;
  padding-bottom: 14px;
  border-bottom: 1px solid var(--line);
  margin-bottom: 12px;
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

.pod-enter-active {
  transition: opacity 220ms ease, transform 280ms var(--ease-out);
}
.pod-enter-from {
  opacity: 0;
  transform: translateY(10px);
}
.pod-leave-active {
  transition: opacity 140ms ease;
}
.pod-leave-to {
  opacity: 0;
}
.pod-move {
  transition: transform 280ms var(--ease-out);
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
