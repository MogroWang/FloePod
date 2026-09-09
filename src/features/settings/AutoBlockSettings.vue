<script setup lang="ts">
import { useSettingsEditor } from "./context";
import { ref } from "vue";
import { open } from "@tauri-apps/plugin-dialog";
import { ipc } from "@/ipc/client";
import type { AutoBlock } from "@/domain/types";
import SettingsRow from "@/components/SettingsRow.vue";
import ToggleSwitch from "@/components/ToggleSwitch.vue";
const { s, save, showToast } = useSettingsEditor();
const blockAppDraft = ref("");

async function saveAutoBlock(update: (current: AutoBlock) => AutoBlock) {
  return save(() => {
    const next = update(s.value.autoBlock);
    return {
      autoBlock: {
        enabled: next.enabled,
        apps: next.apps.map((app) => app.trim()).filter((app) => app.length > 0),
      },
    };
  });
}

/** 模板回调里无法收窄 s 的可空性，改从这里取当前配置。 */
function toggleAutoBlock(enabled: boolean) {
  void saveAutoBlock((current) => ({ ...current, enabled }));
}

/** 与 Rust 侧 exe_matches 对齐：取文件名、小写化、补齐 .exe 后缀后比较。 */
function blockAppKey(raw: string): string {
  const trimmed = raw.trim().replace(/^"+|"+$/g, "");
  const parts = trimmed.split(/[\\/]/).filter((part) => part.length > 0);
  const name = (parts[parts.length - 1] ?? trimmed).toLowerCase();
  return name.endsWith(".exe") ? name : `${name}.exe`;
}

async function addBlockApp(raw: string) {
  const value = raw.trim().replace(/^"+|"+$/g, "");
  if (!value) return;
  const current = s.value?.autoBlock;
  if (!current) return;
  if (current.apps.some((app) => blockAppKey(app) === blockAppKey(value))) {
    showToast("该应用已在列表中");
    return;
  }
  const saved = await saveAutoBlock((latest) => ({
    ...latest,
    apps: latest.apps.some((app) => blockAppKey(app) === blockAppKey(value))
      ? latest.apps
      : [...latest.apps, value],
  }));
  if (saved && blockAppDraft.value === raw) blockAppDraft.value = "";
}

function removeBlockApp(index: number) {
  const current = s.value?.autoBlock;
  if (!current) return;
  const target = current.apps[index];
  if (target === undefined) return;
  void saveAutoBlock((latest) => ({
    ...latest,
    apps: latest.apps.filter((app) => blockAppKey(app) !== blockAppKey(target)),
  }));
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
</script>
<template>
  <div>
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
              <svg
                width="13"
                height="13"
                viewBox="0 0 12 12"
                fill="none"
                stroke="currentColor"
                stroke-width="1.4"
                stroke-linecap="round"
              >
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
  </div>
</template>
