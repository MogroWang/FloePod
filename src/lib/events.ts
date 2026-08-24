/** Tauri 事件名常量与监听封装 */

export const Events = {
  ItemsChanged: "floepod://items-changed",
  SettingsChanged: "floepod://settings-changed",
  PodsChanged: "floepod://pods-changed",
  PanelMode: "floepod://panel-mode",
  PanelShown: "floepod://panel-shown",
  PanelPinned: "floepod://panel-pinned",
  PanelState: "floepod://panel-state",
  PanelHidden: "floepod://panel-hidden",
  CollectClipboard: "floepod://collect-clipboard",
  OpenPanel: "floepod://open-panel",
} as const;

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!("__TAURI_INTERNALS__" in window)) return () => undefined;
  const { listen: rawListen } = await import("@tauri-apps/api/event");
  const unlisten = await rawListen<T>(event, (e) => handler(e.payload));
  return unlisten;
}

/**
 * 仅监听发给当前 WebView 的定向事件。
 * 多匣窗口的显隐、固定、模式和条目刷新必须走这个通道，避免跨匣串扰。
 */
export async function listenCurrent<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (!("__TAURI_INTERNALS__" in window)) return () => undefined;
  const { getCurrentWebviewWindow } = await import("@tauri-apps/api/webviewWindow");
  return getCurrentWebviewWindow().listen<T>(event, (e) => handler(e.payload));
}
