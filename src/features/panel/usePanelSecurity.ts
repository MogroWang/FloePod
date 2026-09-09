import type { PanelContext } from "./context";
import { ipc } from "@/ipc/client";
import { computed, ref, type ComputedRef } from "vue";
import type { SecurityStatus, Pod } from "@/domain/types";

export function usePanelSecurity(context: PanelContext, pod: ComputedRef<Pod | undefined>) {
  const { staging, showToast } = context;
  const securityStatus = ref<SecurityStatus | null>(null);
  const unlocking = ref(false);
  const sensitiveLocked = computed(() =>
    Boolean(pod.value?.security.enabled && securityStatus.value?.locked !== false),
  );
  let revision = 0;
  function applyLockChanged(locked: boolean) {
    revision += 1;
    if (securityStatus.value) securityStatus.value = { ...securityStatus.value, locked };
    if (locked) staging.discardSnapshot();
    else void refreshSecurityStatus();
  }
  async function refreshSecurityStatus() {
    const request = ++revision;
    if (!pod.value?.security.enabled && !pod.value?.rules.expireDays) {
      securityStatus.value = null;
      return;
    }
    try {
      const status = await ipc.getPodSecurityStatus(context.podId());
      if (request !== revision || !context.isMounted()) return;
      securityStatus.value = status;
      if (pod.value?.security.enabled && status.locked) staging.discardSnapshot();
    } catch (error) {
      if (request !== revision || !context.isMounted()) return;
      securityStatus.value = null;
      if (pod.value?.security.enabled) staging.discardSnapshot();
      console.error("security status failed", error);
    }
  }
  async function unlockSensitivePod() {
    if (unlocking.value) return;
    unlocking.value = true;
    try {
      await ipc.unlockSensitivePod(context.podId());
      // Read after the unlock event: an old status response cannot override a subsequent lock.
      await refreshSecurityStatus();
      if (!sensitiveLocked.value) {
        await staging.refresh(context.podId());
        showToast("敏感匣已解锁");
      }
    } catch (error) {
      showToast(`解锁失败：${String(error)}`);
    } finally {
      unlocking.value = false;
    }
  }
  async function lockSensitivePod() {
    try {
      await ipc.lockSensitivePod(context.podId());
      applyLockChanged(true);
    } catch (error) {
      showToast(`锁定失败：${String(error)}`);
    }
  }
  return {
    securityStatus,
    unlocking,
    sensitiveLocked,
    refreshSecurityStatus,
    unlockSensitivePod,
    lockSensitivePod,
    applyLockChanged,
  };
}
