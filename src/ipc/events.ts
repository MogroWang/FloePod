import { listen as listenGlobal } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Events } from "./eventNames";

export { Events };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return listenGlobal<T>(event, ({ payload }) => handler(payload));
}

/** 匣条目和面板事件只在当前 WebView 订阅，不走全局事件总线。 */
export async function listenCurrent<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return getCurrentWebviewWindow().listen<T>(event, ({ payload }) => handler(payload));
}
