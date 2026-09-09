import { listen as listenGlobal } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import type { EventContract } from "./generated";
import { Events } from "./eventNames";

export { Events };

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export async function listen<Name extends keyof EventContract>(
  event: Name,
  handler: (payload: EventContract[Name]) => void,
): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return listenGlobal<EventContract[Name]>(event, ({ payload }) => handler(payload));
}

/** 匣条目和浮动面板事件只在当前 WebView 订阅，不走全局事件总线。 */
export async function listenCurrent<Name extends keyof EventContract>(
  event: Name,
  handler: (payload: EventContract[Name]) => void,
): Promise<() => void> {
  if (!inTauri) return () => undefined;
  return getCurrentWebviewWindow().listen<EventContract[Name]>(event, ({ payload }) =>
    handler(payload),
  );
}
