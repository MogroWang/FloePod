import { listen as listenGlobal } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { Events } from "./eventNames";

export { Events };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function listen<T>(event: string, handler: (payload: T) => void): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return listenGlobal<T>(event, ({ payload }) => handler(payload));
}

/** Pod item/panel events are always subscribed through the current WebView,
 * never through the global app event bus. */
export async function listenCurrent<T>(
  event: string,
  handler: (payload: T) => void,
): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return getCurrentWebviewWindow().listen<T>(event, ({ payload }) => handler(payload));
}
