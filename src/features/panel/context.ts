import type { useStagingStore } from "@/stores/staging";
import type { useSettingsStore } from "@/stores/settings";
export interface PanelContext {
  podId: () => number;
  staging: ReturnType<typeof useStagingStore>;
  settingsStore: ReturnType<typeof useSettingsStore>;
  showToast: (message: string) => void;
  refreshAfterMutation: (label: string) => Promise<boolean>;
  isMounted: () => boolean;
  isBusy: () => boolean;
}
