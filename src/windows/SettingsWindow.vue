<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useToast } from "@/composables/useToast";
import { useSettingsStore } from "@/stores/settings";
import { ipc } from "@/ipc/client";
import BrandMark from "@/components/BrandMark.vue";
import { provideSettingsEditor } from "@/features/settings/context";
import "@/features/settings/settings.css";
import GeneralSettings from "@/features/settings/GeneralSettings.vue";
import AccessibilitySettings from "@/features/settings/AccessibilitySettings.vue";
import AdvancedSettings from "@/features/settings/AdvancedSettings.vue";
import PodSettings from "@/features/settings/PodSettings.vue";
import HotkeySettings from "@/features/settings/HotkeySettings.vue";
import AboutSettings from "@/features/settings/AboutSettings.vue";
import FirstRunSetup from "@/features/settings/FirstRunSetup.vue";
const pageViews = {
  general: GeneralSettings,
  safety: AccessibilitySettings,
  advanced: AdvancedSettings,
  pods: PodSettings,
  hotkeys: HotkeySettings,
  about: AboutSettings,
};
const settingsStore = useSettingsStore();
const s = computed(() => settingsStore.settings);
const page = ref<keyof typeof pageViews>("general");
const { toast, showToast, disposeToast } = useToast(2400);
provideSettingsEditor(showToast);
const loading = ref(true);
const loadError = ref("");
const oobeDone = ref(false);
const firstRun = computed(
  () => !oobeDone.value && !!s.value && (s.value.pods.length === 0 || !s.value.firstRunDone),
);
function finishFirstRun() {
  oobeDone.value = true;
  page.value = "pods";
}
const NAV_ICONS: Record<string, string> = {
  general: '<path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3M2 14h4M10 8h4M18 16h4"/>',
  safety:
    '<circle cx="12" cy="4.5" r="2"/><path d="M4.5 9c2.5.9 5 1.3 7.5 1.3S17 9.9 19.5 9M12 10.3V15m0 0-3 6m3-6 3 6"/>',
  advanced:
    '<path d="M5 8h8M17 8h2M5 16h2M11 16h8"/><circle cx="15" cy="8" r="2"/><circle cx="9" cy="16" r="2"/>',
  pods: '<rect x="3.5" y="3.5" width="17" height="17" rx="3"/><path d="M12 8v8M8 12h8"/>',
  hotkeys:
    '<rect x="3" y="6.5" width="18" height="11" rx="2"/><path d="M7.5 12h.01M12 12h.01M16.5 12h.01M10 15h4"/>',
  about: '<circle cx="12" cy="12" r="8.5"/><path d="M12 11v5M12 8h.01"/>',
};

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

async function loadSettings() {
  loading.value = true;
  loadError.value = "";
  try {
    await settingsStore.listenChanges().catch((error) => {
      console.error("settings listener failed", error);
      showToast("设置实时同步不可用");
    });
    await settingsStore.load();
  } catch (error) {
    console.error("settings initialization failed", error);
    loadError.value = "设置加载失败，请检查数据目录后重试。";
  } finally {
    loading.value = false;
  }
}
onMounted(() => void loadSettings());
onBeforeUnmount(disposeToast);
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
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
          >
            <path d="M2 6h8" />
          </svg>
        </button>
        <button type="button" class="tb-btn close" title="关闭" @click="winClose">
          <svg
            width="12"
            height="12"
            viewBox="0 0 12 12"
            fill="none"
            stroke="currentColor"
            stroke-width="1.4"
            stroke-linecap="round"
          >
            <path d="m3 3 6 6M9 3l-6 6" />
          </svg>
        </button>
      </div>
    </div>

    <div v-if="loading" class="load-state" role="status" aria-live="polite">正在加载设置…</div>
    <div v-else-if="loadError" class="load-state error-state" role="alert">
      <p>{{ loadError }}</p>
      <button type="button" class="btn primary" @click="loadSettings">重试</button>
    </div>

    <template v-else-if="s">
      <FirstRunSetup v-if="firstRun" @complete="finishFirstRun" />
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
              <KeepAlive>
                <component :is="pageViews[page]" :key="page" />
              </KeepAlive>
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
