import { computed, inject, provide, ref, type InjectionKey, type Ref } from "vue";
import { useSettingsStore, type SettingsPatchSource } from "@/stores/settings";
import type { Pod } from "@/domain/types";
type Context = { notify: (message: string) => void; selectedPodId: Ref<number | null> };
const key: InjectionKey<Context> = Symbol("settings-editor");
export function provideSettingsEditor(notify: Context["notify"]) {
  provide(key, { notify, selectedPodId: ref(null) });
}
export function useSettingsEditor() {
  const context = inject(key);
  if (!context) throw new Error("设置页面未初始化");
  const settingsStore = useSettingsStore();
  const s = computed(() => settingsStore.settings!);
  const monitors = computed(() => settingsStore.monitors);
  const showToast = context.notify;
  async function save(source: SettingsPatchSource): Promise<boolean> {
    try {
      await settingsStore.save(source);
      return true;
    } catch (error) {
      console.error("settings save failed", error);
      showToast("保存失败，请重试");
      return false;
    }
  }
  async function savePod(id: number, patch: Partial<Pod>): Promise<boolean> {
    try {
      await settingsStore.updatePod(id, patch);
      return true;
    } catch (error) {
      console.error("pod save failed", error);
      showToast("保存失败，请重试");
      return false;
    }
  }
  return {
    settingsStore,
    s,
    monitors,
    showToast,
    save,
    savePod,
    selectedPodId: context.selectedPodId,
  };
}
export function useSelectedPod() {
  const { s, selectedPodId } = useSettingsEditor();
  const selectedPod = computed(
    () => s.value.pods.find((p) => p.id === selectedPodId.value) ?? s.value.pods[0] ?? null,
  );
  const selectedPodList = computed(() => (selectedPod.value ? [selectedPod.value] : []));
  return {
    selectedPod,
    selectedPodList,
    selectPod: (id: number) => {
      selectedPodId.value = id;
    },
  };
}
