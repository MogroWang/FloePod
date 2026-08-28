import { ref } from "vue";

/**
 * 轻量 toast：同一时间只显示一条，重复调用重置计时。
 * `enabled` 用于窗口卸载后不再更新状态的场景（传入如 `isMounted`）。
 */
export function useToast(durationMs = 2200, enabled: () => boolean = () => true) {
  const toast = ref("");
  let timer: number | undefined;

  function showToast(message: string) {
    if (!enabled()) return;
    toast.value = message;
    window.clearTimeout(timer);
    timer = window.setTimeout(() => (toast.value = ""), durationMs);
  }

  function disposeToast() {
    window.clearTimeout(timer);
    toast.value = "";
  }

  return { toast, showToast, disposeToast };
}
