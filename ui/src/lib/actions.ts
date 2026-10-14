import { invoke } from "@tauri-apps/api/core";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { player, ASPECT_RATIOS, CROPS, PLAYLIST_EXTENSIONS, SUBTITLE_EXTENSIONS } from "./player.svelte";
import { ui } from "./ui.svelte";

/** App-level operations the actions need (provided by the main window). */
export interface Host {
  openFiles: () => void;
  openFolder: () => void;
  toggleFullscreen: () => void;
}

export interface Action {
  id: string;
  label: string;
  group: "Playback" | "Audio" | "Video" | "Subtitles" | "App";
  /** Default binding, VLC-style. "" means none. */
  keys: string;
  run: (host: Host) => unknown;
}

function cycle<T>(list: T[], current: T): T {
  return list[(list.indexOf(current) + 1) % list.length];
}

const s = () => player.state;

/**
 * Every action that can have a shortcut. Ids match the menu bar items in
 * src-tauri/src/menu.rs.
 */
export const ACTIONS: Action[] = [
  { id: "playPause", label: "Play / pause", group: "Playback", keys: "Space", run: () => player.togglePause() },
  { id: "stop", label: "Stop", group: "Playback", keys: "Cmd+.", run: () => player.stop() },
  { id: "next", label: "Next", group: "Playback", keys: "N", run: () => player.next() },
  { id: "previous", label: "Previous", group: "Playback", keys: "P", run: () => player.previous() },
  { id: "seekForward", label: "Jump forward 5 s", group: "Playback", keys: "ArrowRight", run: () => player.seekBy(5_000) },
  { id: "seekBack", label: "Jump back 5 s", group: "Playback", keys: "ArrowLeft", run: () => player.seekBy(-5_000) },
  { id: "seekForwardLong", label: "Jump forward 30 s", group: "Playback", keys: "Shift+ArrowRight", run: () => player.seekBy(30_000) },
  { id: "seekBackLong", label: "Jump back 30 s", group: "Playback", keys: "Shift+ArrowLeft", run: () => player.seekBy(-30_000) },
  { id: "nextFrame", label: "Next frame", group: "Playback", keys: "E", run: () => player.nextFrame() },
  { id: "jumpToTime", label: "Jump to time…", group: "Playback", keys: "Cmd+T", run: () => player.active && (ui.dialog = "jump") },
  { id: "abLoop", label: "A–B loop", group: "Playback", keys: "Shift+L", run: () => player.abLoopStep() },
  { id: "nextChapter", label: "Next chapter", group: "Playback", keys: "Shift+N", run: () => player.stepChapter(1) },
  { id: "previousChapter", label: "Previous chapter", group: "Playback", keys: "Shift+P", run: () => player.stepChapter(-1) },
  { id: "nextTitle", label: "Next title", group: "Playback", keys: "", run: () => player.stepTitle(1) },
  { id: "previousTitle", label: "Previous title", group: "Playback", keys: "", run: () => player.stepTitle(-1) },
  { id: "discMenu", label: "Disc menu", group: "Playback", keys: "Shift+M", run: () => player.navigate("popup") },
  { id: "addBookmark", label: "Add bookmark", group: "Playback", keys: "Cmd+B", run: () => player.addBookmark() },
  { id: "sleepTimer", label: "Sleep timer", group: "Playback", keys: "", run: () => player.cycleSleepTimer() },
  { id: "faster", label: "Faster", group: "Playback", keys: "]", run: () => player.setRate(s().rate + 0.25) },
  { id: "slower", label: "Slower", group: "Playback", keys: "[", run: () => player.setRate(s().rate - 0.25) },
  { id: "normalSpeed", label: "Normal speed", group: "Playback", keys: "=", run: () => player.setRate(1) },

  {
    id: "volumeUp", label: "Increase volume", group: "Audio", keys: "ArrowUp",
    run: () => { player.setVolume(s().volume + 5); player.notify("osd", `Volume ${Math.min(200, s().volume)}%`); },
  },
  {
    id: "volumeDown", label: "Decrease volume", group: "Audio", keys: "ArrowDown",
    run: () => { player.setVolume(s().volume - 5); player.notify("osd", `Volume ${Math.max(0, s().volume)}%`); },
  },
  {
    id: "mute", label: "Mute", group: "Audio", keys: "M",
    run: () => { player.notify("osd", s().muted ? "Unmuted" : "Muted"); player.setMuted(!s().muted); },
  },
  { id: "cycleAudio", label: "Next audio track", group: "Audio", keys: "B", run: () => player.cycle("audio") },
  { id: "audioDelayUp", label: "Increase audio delay", group: "Audio", keys: "K", run: () => player.setAudioDelay(s().audioDelay + 50) },
  { id: "audioDelayDown", label: "Decrease audio delay", group: "Audio", keys: "J", run: () => player.setAudioDelay(s().audioDelay - 50) },

  { id: "fullscreen", label: "Toggle fullscreen", group: "Video", keys: "F", run: (h) => h.toggleFullscreen() },
  { id: "snapshot", label: "Take snapshot", group: "Video", keys: "Shift+S", run: () => player.snapshot() },
  { id: "cycleAspect", label: "Next aspect ratio", group: "Video", keys: "A", run: () => player.setAspect(cycle(ASPECT_RATIOS, player.aspect)) },
  { id: "cycleCrop", label: "Next crop", group: "Video", keys: "C", run: () => player.setCrop(cycle(CROPS, player.crop)) },
  { id: "deinterlace", label: "Toggle deinterlace", group: "Video", keys: "D", run: () => player.setDeinterlace(!player.deinterlace) },

  { id: "toggleSubtitles", label: "Subtitles on / off", group: "Subtitles", keys: "V", run: () => player.toggleSubtitles() },
  {
    id: "addSubtitle", label: "Add subtitle file…", group: "Subtitles", keys: "",
    run: async () => {
      const file = await openDialog({ multiple: false, title: "Add subtitles", filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }] });
      if (typeof file === "string") player.addSubtitle(file);
    },
  },
  { id: "subtitleDelayUp", label: "Increase subtitle delay", group: "Subtitles", keys: "H", run: () => player.setSubtitleDelay(s().subtitleDelay + 50) },
  { id: "subtitleDelayDown", label: "Decrease subtitle delay", group: "Subtitles", keys: "G", run: () => player.setSubtitleDelay(s().subtitleDelay - 50) },

  { id: "open", label: "Open files", group: "App", keys: "Cmd+O", run: (h) => h.openFiles() },
  { id: "openFolder", label: "Add folder", group: "App", keys: "Shift+Cmd+O", run: (h) => h.openFolder() },
  { id: "openUrl", label: "Open network stream", group: "App", keys: "Cmd+N", run: () => (ui.dialog = "url") },
  { id: "openDisc", label: "Open disc", group: "App", keys: "Cmd+D", run: () => (ui.dialog = "disc") },
  { id: "openCapture", label: "Open capture device", group: "App", keys: "", run: () => (ui.dialog = "capture") },
  { id: "savePlaylist", label: "Save playlist", group: "App", keys: "Cmd+S", run: () => savePlaylist() },
  { id: "queue", label: "Up next", group: "App", keys: "Cmd+L", run: () => (ui.playlistOpen = !ui.playlistOpen) },
  { id: "help", label: "Keyboard shortcuts", group: "App", keys: "?", run: () => (ui.helpOpen = !ui.helpOpen) },
];

/** Asks where to save the queue, as M3U, PLS or XSPF. */
export async function savePlaylist() {
  if (!player.playlist.items.length) return player.notify("info", "The queue is empty");
  const path = await saveDialog({
    title: "Save playlist",
    defaultPath: "Playlist.m3u8",
    filters: [{ name: "Playlist", extensions: PLAYLIST_EXTENSIONS }],
  });
  if (path) player.savePlaylist(path);
}

export const ACTION_BY_ID = new Map(ACTIONS.map((a) => [a.id, a]));

/** Fixed shortcuts that can't be changed, shown for reference. */
export const FIXED_SHORTCUTS: [string, string][] = [
  ["Cmd+,", "Settings"],
  ["Escape", "Close panel / exit fullscreen"],
  ["0–9", "Jump to 0–90%"],
  ["Arrows, Return", "Move around DVD and Blu-ray menus"],
  ["Double-click", "Toggle fullscreen"],
];

/** The binding currently in effect for each action. */
export function effectiveBindings(overrides: Record<string, string>): Record<string, string> {
  return Object.fromEntries(ACTIONS.map((a) => [a.id, overrides[a.id] ?? a.keys]));
}

/** Converts a key event to a binding string like "Shift+Cmd+O", or null for bare modifiers. */
export function eventBinding(e: KeyboardEvent): string | null {
  if (["Meta", "Shift", "Alt", "Control", "CapsLock"].includes(e.key)) return null;
  const letter = /^Key[A-Z]$/.test(e.code);
  const digit = /^Digit\d$/.test(e.code);
  let key: string;
  if (letter) key = e.code.slice(3);
  else if (digit && !e.shiftKey) key = e.code.slice(5);
  else if (e.key === " ") key = "Space";
  else key = e.key.length === 1 ? e.key.toUpperCase() : e.key;
  const mods: string[] = [];
  if (e.ctrlKey) mods.push("Ctrl");
  if (e.altKey) mods.push("Alt");
  // For symbols, Shift is already reflected in the character ("?" not "Shift+/").
  if (e.shiftKey && (letter || digit || key.length > 1)) mods.push("Shift");
  if (e.metaKey) mods.push("Cmd");
  return [...mods, key].join("+");
}

const SYMBOLS: Record<string, string> = {
  Cmd: "⌘", Shift: "⇧", Alt: "⌥", Ctrl: "⌃",
  ArrowLeft: "←", ArrowRight: "→", ArrowUp: "↑", ArrowDown: "↓", Escape: "Esc", Space: "Space",
};

/** "Shift+Cmd+O" → "⇧⌘O", the macOS way. */
export function formatBinding(binding: string) {
  if (!binding) return "";
  const parts = binding.split("+");
  const key = parts.pop()!;
  return parts.map((p) => SYMBOLS[p] ?? p).join("") + (SYMBOLS[key] ?? key);
}

/** Tells the menu bar which bindings to show as accelerators. */
export function syncMenu(bindings: Record<string, string>) {
  return invoke("set_menu_shortcuts", { keys: bindings }).catch(() => {});
}
