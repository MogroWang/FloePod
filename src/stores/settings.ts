import { defineStore } from "pinia";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { resolveTheme } from "@/domain/settings";
import type { MonitorInfo, Pod, Settings, ThemeMode } from "@/domain/types";
import { ipc } from "@/ipc/client";
import { Events, listen } from "@/ipc/events";

let changesListening = false;
let changesListenPromise: Promise<void> | null = null;
let systemThemeWatching = false;

function applyDocumentTheme(theme: "light" | "dark") {
  const root = document.documentElement;
  root.classList.toggle("dark", theme === "dark");
  root.classList.toggle("light", theme === "light");
  root.style.colorScheme = theme;
  root.classList.add("theme-ready");
}

async function applyNativeTheme(mode: ThemeMode) {
  if (!("__TAURI_INTERNALS__" in window)) return;
  try {
    await getCurrentWindow().setTheme(mode === "system" ? null : mode);
  } catch (err) {
    console.warn("native theme sync failed", err);
  }
}

export const useSettingsStore = defineStore("settings", {
  state: () => ({
    settings: null as Settings | null,
    monitors: [] as MonitorInfo[],
    systemDark: window.matchMedia("(prefers-color-scheme: dark)").matches,
    dark: window.matchMedia("(prefers-color-scheme: dark)").matches,
    bootstrapSeq: 0,
  }),

  getters: {
    configured: (s) => !!s.settings?.pods.length,
  },

  actions: {
    async load() {
      const request = ++this.bootstrapSeq;
      const boot = await ipc.getBootstrap();
      if (request !== this.bootstrapSeq) return;
      this.monitors = boot.monitors;
      this.apply(boot.settings);
      void this.watchSystemTheme();
    },

    apply(settings: Settings) {
      this.settings = settings;
      const theme = resolveTheme(settings.theme, this.systemDark);
      this.dark = theme === "dark";
      applyDocumentTheme(theme);
      void applyNativeTheme(settings.theme);
    },

    async refreshPods() {
      const request = ++this.bootstrapSeq;
      const boot = await ipc.getBootstrap();
      if (request !== this.bootstrapSeq) return;
      this.monitors = boot.monitors;
      this.apply(boot.settings);
    },

    async save(patch: Partial<Settings>) {
      const request = ++this.bootstrapSeq;
      const next = await ipc.saveSettings(patch);
      if (request === this.bootstrapSeq) this.apply(next);
      return next;
    },

    pod(id: number): Pod | undefined {
      return this.settings?.pods.find((p) => p.id === id);
    },

    async watchSystemTheme() {
      if (systemThemeWatching) return;
      systemThemeWatching = true;

      const update = (dark: boolean) => {
        this.systemDark = dark;
        if (this.settings?.theme === "system") {
          this.dark = dark;
          applyDocumentTheme(dark ? "dark" : "light");
        }
      };
      const media = window.matchMedia("(prefers-color-scheme: dark)");
      media.addEventListener("change", (event) => {
        // WebView2 在强制深/浅主题时也可能改变 media query；
        // Tauri 环境下非跟随模式以真正的系统主题事件为准。
        if (!ipc.inTauri || this.settings?.theme === "system") update(event.matches);
      });

      // Tauri 的原生主题事件作为 WebView2 media query 的互补来源。
      if (ipc.inTauri) {
        try {
          const current = getCurrentWindow();
          const initial = await current.theme();
          if (initial && this.settings?.theme === "system") update(initial === "dark");
          await current.onThemeChanged(({ payload }) => {
            if (this.settings?.theme === "system") update(payload === "dark");
          });
        } catch (err) {
          console.warn("system theme listener unavailable", err);
        }
      }
    },

    async listenChanges() {
      if (changesListening) return;
      if (changesListenPromise) return changesListenPromise;
      changesListenPromise = (async () => {
        const registrations = await Promise.allSettled([
          listen<Settings>(Events.SettingsChanged, (settings) => {
            this.bootstrapSeq += 1;
            this.apply(settings);
          }),
          listen<void>(Events.PodsChanged, () => {
            void this.refreshPods().catch((err) => console.error("pod refresh failed", err));
          }),
        ]);
        const failed = registrations.find(
          (result): result is PromiseRejectedResult => result.status === "rejected",
        );
        if (failed) {
          for (const result of registrations) {
            if (result.status === "fulfilled") result.value();
          }
          throw failed.reason;
        }
        changesListening = true;
      })();
      try {
        await changesListenPromise;
      } finally {
        changesListenPromise = null;
      }
    },
  },
});
