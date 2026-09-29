<script setup lang="ts">
import { computed, onMounted, ref } from "vue";
import { openPath, openUrl } from "@tauri-apps/plugin-opener";
import { open as openDirectory } from "@tauri-apps/plugin-dialog";
import { useSettingsEditor } from "./context";
import { ipc } from "@/ipc/client";
import { formatSize } from "@/lib/format";
import BrandMark from "@/components/BrandMark.vue";

const { s, showToast } = useSettingsEditor();

/* 数据占用：后端递归统计数据目录（数据库、撤销区、日志）。 */
const usage = ref<number | null>(null);
const usageText = computed(() => (usage.value === null ? "统计中…" : formatSize(usage.value)));
onMounted(() => {
  ipc
    .getDataUsage()
    .then((result) => (usage.value = result.bytes))
    .catch(() => (usage.value = null));
});

/* 检查更新：查询 GitHub Releases 最新版本号并比较。 */
const RELEASES_URL = "https://github.com/MogroWang/FloePod/releases/latest";
const RELEASES_API = "https://api.github.com/repos/MogroWang/FloePod/releases/latest";
const checking = ref(false);
const updateState = ref<"idle" | "latest" | "available" | "failed">("idle");
const latestVersion = ref("");

function compareVersions(a: string, b: string): number {
  const left = a.split(".").map(Number);
  const right = b.split(".").map(Number);
  for (let i = 0; i < 3; i += 1) {
    const diff = (left[i] || 0) - (right[i] || 0);
    if (diff) return diff;
  }
  return 0;
}

async function checkUpdates() {
  if (checking.value) return;
  checking.value = true;
  try {
    const response = await fetch(RELEASES_API, {
      headers: { Accept: "application/vnd.github+json" },
    });
    if (!response.ok) throw new Error(`HTTP ${response.status}`);
    const release = (await response.json()) as { tag_name?: string };
    const tag = (release.tag_name ?? "").replace(/^v/, "");
    if (!tag) throw new Error("empty tag");
    latestVersion.value = tag;
    updateState.value = compareVersions(tag, s.value.version) > 0 ? "available" : "latest";
  } catch (error) {
    console.error("check update failed", error);
    updateState.value = "failed";
  } finally {
    checking.value = false;
  }
}

const updateText = computed(() => {
  if (checking.value) return "正在检查…";
  switch (updateState.value) {
    case "latest":
      return "已是最新版本";
    case "available":
      return `发现新版本 ${latestVersion.value}`;
    case "failed":
      return "检查失败，请稍后重试";
    default:
      return "";
  }
});

/* 更改数据位置：选择新目录 → 复制数据 → 重启生效。 */
const migrating = ref(false);
const pendingRestart = ref(false);

async function chooseDataDir() {
  if (migrating.value) return;
  try {
    const selection = await openDirectory({ directory: true, title: "选择新的数据位置" });
    if (typeof selection !== "string") return;
    migrating.value = true;
    await ipc.changeDataDir(selection);
    pendingRestart.value = true;
    showToast("数据已迁移，重启 FloePod 后生效");
  } catch (error) {
    console.error("change data dir failed", error);
    showToast(`迁移失败：${String(error)}`);
  } finally {
    migrating.value = false;
  }
}

async function openDataDir() {
  try {
    await openPath(s.value.dataDir);
  } catch (error) {
    showToast(`无法打开数据文件夹：${String(error)}`);
  }
}

async function restartNow() {
  try {
    await ipc.restartApp();
  } catch (error) {
    showToast(`重启失败：${String(error)}`);
  }
}
</script>
<template>
  <div>
    <div class="about-hero">
      <BrandMark :size="48" class="about-brand" />
      <div class="about-ver">版本 {{ s.version }}</div>
    </div>
    <p class="about-text">
      把文件拖到屏幕边缘的匣里集中保管，需要时再拖出去继续使用。<br />
      不联网，不收集数据——所有内容都只保存在你自己的电脑上。
    </p>
    <div class="about-meta">
      <div class="about-row">
        <span class="about-key">数据位置</span>
        <span class="about-val about-val-path">
          <span class="about-path">{{ s.dataDir }}</span>
          <span class="about-row-actions">
            <button type="button" class="btn" @click="openDataDir">打开</button>
            <button type="button" class="btn" :disabled="migrating" @click="chooseDataDir">
              {{ migrating ? "迁移中…" : "更改…" }}
            </button>
          </span>
        </span>
      </div>
      <div class="about-row">
        <span class="about-key">占用空间</span>
        <span class="about-val">{{ usageText }}</span>
      </div>
      <div class="about-row">
        <span class="about-key">匣的数量</span>
        <span class="about-val">{{ s.pods.length }} 个</span>
      </div>
    </div>
    <p v-if="pendingRestart" class="about-restart-note">
      数据已迁移到新位置。重启 FloePod 后生效，当前会话仍使用旧位置。
    </p>
    <div class="reset-line about-update-line">
      <button v-if="pendingRestart" type="button" class="btn primary" @click="restartNow">
        立即重启
      </button>
      <button type="button" class="btn" :disabled="checking" @click="checkUpdates">检查更新</button>
      <span v-if="updateText" class="about-update-result" role="status">{{ updateText }}</span>
      <button
        v-if="updateState === 'available'"
        type="button"
        class="btn primary"
        @click="openUrl(RELEASES_URL)"
      >
        前往下载
      </button>
    </div>
  </div>
</template>
