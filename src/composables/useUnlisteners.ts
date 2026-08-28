import { onBeforeUnmount } from "vue";

/**
 * 事件监听注册表：挂载期间注册的监听在卸载时统一解绑；
 * 卸载之后才解析出来的监听立即解绑，避免竞态泄漏。
 */
export function useUnlisteners() {
  const unlisteners: Array<() => void> = [];
  let mounted = true;

  function retainUnlistener(unlisten: () => void) {
    if (mounted) unlisteners.push(unlisten);
    else unlisten();
  }

  function disposeUnlisteners() {
    mounted = false;
    unlisteners.splice(0).forEach((unlisten) => unlisten());
  }

  function isMounted() {
    return mounted;
  }

  onBeforeUnmount(disposeUnlisteners);

  return { retainUnlistener, disposeUnlisteners, isMounted };
}
