<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { nextTick, reactive, ref } from "vue";
import { ask } from "@tauri-apps/plugin-dialog";
import type { Edge, DropAction, Material, Pod } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { useSelectedPod } from "./context";
import { folderPicker } from "./folderPicker";
import { usePodDrafts } from "./usePodDrafts";
import { EDGES, DROP_ACTIONS, MATERIALS } from "./options";
import SegmentedControl from "@/components/SegmentedControl.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
import RangeSlider from "@/components/RangeSlider.vue";
import PodEdgeDiagram from "@/components/PodEdgeDiagram.vue";
const { s, settingsStore, monitors, savePod, showToast, selectedPodId } = useSettingsEditor();
const { selectedPod, selectedPodList, selectPod } = useSelectedPod();
const pickFolder = folderPicker(showToast);
const deletingPodIds = reactive(new Set<number>());
const podEnabledBusyIds = reactive(new Set<number>());
const {
  podNumberValue,
  previewPodNumber,
  commitPodNumber,
  podHexColorValue,
  colorDraftValue,
  previewPodColor,
  hasColorDraft,
  confirmPodColor,
  clearPodColor,
} = usePodDrafts(savePod);
function monitorLabel(pod: Pod): string {
  if (!pod.monitor) return "主显示器";
  return monitors.value.find((m) => m.name === pod.monitor)?.label ?? pod.monitor;
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

async function changePodFolder(pod: Pod) {
  const folder = await pickFolder();
  if (folder) await savePod(pod.id, { stagingFolder: folder });
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
    // 未在表单中选择的字段交给后端 Pod::default，避免默认值与校验范围漂移。
    const pod = await ipc.createPod({
      name: addDraft.name.trim() || `匣 ${n + 1}`,
      edge,
      stagingFolder: folder,
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
</script>
<template>
  <div>
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

    <div
      v-for="pod in selectedPodList"
      :key="pod.id"
      class="pod-card"
      :class="{ off: !pod.enabled }"
    >
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
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
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
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              stroke-width="1.7"
              stroke-linecap="round"
              stroke-linejoin="round"
            >
              <path
                d="M5 7h14M9 7V5a1 1 0 0 1 1-1h4a1 1 0 0 1 1 1v2m3 0-1 13a1.5 1.5 0 0 1-1.5 1.4h-7A1.5 1.5 0 0 1 6.5 20L5.5 7"
              />
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
              <SegmentedControl
                :options="EDGES"
                :model-value="pod.edge"
                @update:model-value="(v) => savePod(pod.id, { edge: v as Edge })"
              />
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
            <span class="flabel">浮动条宽度</span>
            <div class="fctrl">
              <RangeSlider
                :value="podNumberValue(pod, 'barWidth')"
                :min="28"
                :max="96"
                :step="2"
                aria-label="浮动条宽度"
                @update:value="(v) => previewPodNumber(pod.id, 'barWidth', v)"
                @commit="(v) => commitPodNumber(pod, 'barWidth', v)"
              />
              <span class="fval">{{ podNumberValue(pod, "barWidth") }}px</span>
            </div>
          </div>
          <div class="frow">
            <span class="flabel">浮动条长度</span>
            <div class="fctrl">
              <RangeSlider
                :value="podNumberValue(pod, 'barLength')"
                :min="100"
                :max="500"
                :step="10"
                aria-label="浮动条长度"
                @update:value="(v) => previewPodNumber(pod.id, 'barLength', v)"
                @commit="(v) => commitPodNumber(pod, 'barLength', v)"
              />
              <span class="fval">{{ podNumberValue(pod, "barLength") }}px</span>
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
            <span class="flabel">填充色不透明度</span>
            <div class="fctrl">
              <RangeSlider
                :value="podNumberValue(pod, 'opacity')"
                :min="0.1"
                :max="1"
                :step="0.01"
                aria-label="填充色不透明度"
                @update:value="(v) => previewPodNumber(pod.id, 'opacity', v)"
                @commit="(v) => commitPodNumber(pod, 'opacity', v)"
              />
              <span class="fval">{{ Math.round(podNumberValue(pod, "opacity") * 100) }}%</span>
            </div>
          </div>
          <div class="frow">
            <span class="flabel">浮动条填充色</span>
            <div class="fctrl">
              <label class="color-field">
                <input
                  type="color"
                  class="color-input"
                  :value="podHexColorValue(colorDraftValue(pod, 'barColor'))"
                  aria-label="浮动条填充色"
                  @input="
                    (e) => previewPodColor(pod, 'barColor', (e.target as HTMLInputElement).value)
                  "
                />
                <span class="color-text">{{ colorDraftValue(pod, "barColor") || "跟随主题" }}</span>
              </label>
              <button
                v-if="hasColorDraft(pod, 'barColor')"
                type="button"
                class="btn primary"
                @click="confirmPodColor(pod, 'barColor')"
              >
                确定
              </button>
              <button
                v-if="pod.barColor"
                type="button"
                class="btn ghost"
                @click="clearPodColor(pod, 'barColor')"
              >
                重置
              </button>
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
                  @input="
                    (e) => previewPodColor(pod, 'borderColor', (e.target as HTMLInputElement).value)
                  "
                />
                <span class="color-text">{{
                  colorDraftValue(pod, "borderColor") || "跟随主题"
                }}</span>
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
              <span class="fval"
                >{{ Math.round(podNumberValue(pod, "borderOpacity") * 100) }}%</span
              >
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
              <span class="fval"
                >{{
                  (podNumberValue(pod, "stealthDelayMs") / 1000).toFixed(2).replace(/\.?0+$/, "")
                }}s</span
              >
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
                :min="410"
                :max="600"
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
              <SegmentedControl
                :options="MATERIALS"
                :model-value="pod.panelMaterial"
                @update:model-value="(v) => savePod(pod.id, { panelMaterial: v as Material })"
              />
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
                  @input="
                    (e) => previewPodColor(pod, 'panelColor', (e.target as HTMLInputElement).value)
                  "
                />
                <span class="color-text">{{
                  colorDraftValue(pod, "panelColor") || "跟随主题"
                }}</span>
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
              <SegmentedControl
                :options="DROP_ACTIONS"
                :model-value="pod.dropAction"
                @update:model-value="(v) => savePod(pod.id, { dropAction: v as DropAction })"
              />
            </div>
          </div>
          <div class="frow folder-row">
            <span class="flabel">暂存文件夹</span>
            <div class="fctrl folder-line">
              <input
                :value="pod.stagingFolder"
                class="input mono"
                readonly
                :title="pod.stagingFolder"
                placeholder="未选择"
              />
              <button type="button" class="btn" @click="changePodFolder(pod)">选择…</button>
              <button
                v-if="pod.stagingFolder"
                type="button"
                class="btn ghost"
                @click="openPodFolder(pod)"
              >
                打开
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
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
              <input
                :value="addDraft.folder"
                class="input mono"
                readonly
                placeholder="选择存放暂存文件的文件夹"
              />
              <button type="button" class="btn" :disabled="addPodCreating" @click="chooseAddFolder">
                选择…
              </button>
            </div>
          </div>
          <div class="modal-actions">
            <button
              type="button"
              class="btn ghost"
              :disabled="addPodCreating"
              @click="cancelAddDialog"
            >
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
  </div>
</template>
