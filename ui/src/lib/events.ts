import { listen, type EventCallback, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";

/**
 * Listens for events sent to this window, and for broadcasts. A plain `listen`
 * also receives events the backend sent to other windows, which would show
 * one player's state in every window.
 */
export function listenHere<T>(event: string, handler: EventCallback<T>): Promise<UnlistenFn> {
  return listen<T>(event, handler, { target: getCurrentWebview().label });
}
