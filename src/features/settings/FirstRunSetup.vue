<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { onMounted, ref } from "vue";
import type { Edge, Material, Pod, ThemeMode } from "@/domain/types";
import { normalizeWindowsPathKey } from "@/domain/settings";
import { ipc } from "@/ipc/client";
import { withTimeout } from "@/lib/timeout";
import { folderPicker } from "./folderPicker";
import { EDGES, THEMES, MATERIALS } from "./options";
import BrandMark from "@/components/BrandMark.vue";
import SegmentedControl from "@/components/SegmentedControl.vue";
import RangeSlider from "@/components/RangeSlider.vue";
const { s, settingsStore, monitors, showToast } = useSettingsEditor();
const emit = defineEmits<{ complete: [] }>();
const pickFolder = folderPicker(showToast);
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

function appLog(msg: string) {
  void ipc.appLog(msg).catch((err) => console.warn("app log failed", err));
}

async function chooseOobeFolder() {
  const folder = await pickFolder();
  if (folder) oobe.value.folder = folder;
}

function oobePodConfig(): Partial<Pod> {
  // 只提交向导中的用户选择；面板宽度等默认值统一由后端补齐。
  return {
    name: oobe.value.name || "我的匣",
    edge: oobe.value.edge,
    monitor: oobe.value.monitor,
    stagingFolder: oobe.value.folder,
    opacity: Number(oobe.value.opacity),
    panelMaterial: oobe.value.material,
    panelOpacity: Number(oobe.value.opacity),
  };
}

function findOobePodByFolder(): Pod | undefined {
  const folderKey = normalizeWindowsPathKey(oobe.value.folder);
  if (!folderKey) return undefined;
  return s.value?.pods.find((pod) => normalizeWindowsPathKey(pod.stagingFolder) === folderKey);
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
      settingsStore.save({ theme: oobe.value.theme, firstRunDone: true }),
      "saveSettings",
    );
    appLog("finishOobe: saveSettings 完成");
    await withTimeout(settingsStore.refreshPods(), "refreshPods");
    appLog("finishOobe: refreshPods 完成");
    emit("complete");
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

onMounted(() => {
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
});
</script>
<template>
  <div class="oobe">
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
              <input
                :value="oobe.folder"
                class="input mono"
                readonly
                placeholder="选择存放暂存文件的文件夹"
              />
              <button type="button" class="btn" @click="chooseOobeFolder">选择…</button>
            </div>
          </div>
        </div>
        <div class="oobe-actions">
          <button type="button" class="btn ghost" @click="oobeStep = 1">上一步</button>
          <button type="button" class="btn primary" :disabled="!oobe.folder" @click="nextFromStep2">
            下一步
          </button>
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
            <span>填充色不透明度</span>
            <div class="slider-line">
              <RangeSlider
                :value="oobe.opacity"
                :min="0.1"
                :max="1"
                :step="0.01"
                aria-label="填充色不透明度"
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
</template>
