import { player, ASPECT_RATIOS, CROPS } from "./player.svelte";
import { ui } from "./ui.svelte";

export interface Actions {
  openFiles: () => void;
  openFolder: () => void;
  toggleFullscreen: () => void;
  exitFullscreen: () => void;
}

/** Shortcut list shown in the help overlay. Keys follow VLC where possible. */
export const SHORTCUTS: [string, string][] = [
  ["Space", "Play / pause"],
  ["← / →", "Back / forward 5 s"],
  ["⇧← / ⇧→", "Back / forward 30 s"],
  ["↑ / ↓", "Volume up / down"],
  ["M", "Mute"],
  ["F, double-click", "Fullscreen"],
  ["N / P", "Next / previous"],
  ["[ / ] / =", "Slower / faster / normal speed"],
  ["E", "Next frame"],
  ["0–9", "Jump to 0–90%"],
  ["B / V", "Cycle audio / subtitles on-off"],
  ["A / C", "Cycle aspect ratio / crop"],
  ["G / H", "Subtitle delay −/+ 50 ms"],
  ["J / K", "Audio delay −/+ 50 ms"],
  ["⇧S", "Snapshot"],
  ["⌘O / ⇧⌘O", "Open files / add folder"],
  ["⌘N", "Open network stream"],
  ["⌘L", "Up next"],
  ["?", "Show shortcuts"],
];

function cycleValue<T>(list: T[], current: T): T {
  return list[(list.indexOf(current) + 1) % list.length];
}

export function handleKey(e: KeyboardEvent, actions: Actions): boolean {
  const target = e.target as HTMLElement | null;
  if (target && (target.tagName === "INPUT" && (target as HTMLInputElement).type === "text")) return false;
  if (ui.urlOpen) return false;

  const s = player.state;
  const cmd = e.metaKey || e.ctrlKey;

  if (cmd) {
    switch (e.key.toLowerCase()) {
      case "o":
        if (e.shiftKey) actions.openFolder();
        else actions.openFiles();
        return true;
      case "n":
        ui.urlOpen = true;
        return true;
      case "l":
        ui.playlistOpen = !ui.playlistOpen;
        return true;
    }
    return false;
  }

  switch (e.key) {
    case " ":
      player.togglePause();
      return true;
    case "ArrowRight":
      player.seekBy(e.shiftKey ? 30_000 : 5_000);
      return true;
    case "ArrowLeft":
      player.seekBy(e.shiftKey ? -30_000 : -5_000);
      return true;
    case "ArrowUp":
      player.setVolume(s.volume + 5);
      player.notify("osd", `Volume ${Math.min(200, s.volume + 5)}%`);
      return true;
    case "ArrowDown":
      player.setVolume(s.volume - 5);
      player.notify("osd", `Volume ${Math.max(0, s.volume - 5)}%`);
      return true;
    case "Escape":
      if (ui.settingsOpen) ui.settingsOpen = false;
      else if (ui.playlistOpen) ui.playlistOpen = false;
      else if (ui.helpOpen) ui.helpOpen = false;
      else actions.exitFullscreen();
      return true;
    case "?":
      ui.helpOpen = !ui.helpOpen;
      return true;
    case "S":
      if (e.shiftKey) player.snapshot();
      return e.shiftKey;
  }

  switch (e.key.toLowerCase()) {
    case "f":
      actions.toggleFullscreen();
      return true;
    case "m":
      player.setMuted(!s.muted);
      player.notify("osd", s.muted ? "Unmuted" : "Muted");
      return true;
    case "n":
      player.next();
      return true;
    case "p":
      player.previous();
      return true;
    case "[":
      player.setRate(s.rate - 0.25);
      return true;
    case "]":
      player.setRate(s.rate + 0.25);
      return true;
    case "=":
      player.setRate(1);
      return true;
    case "e":
      player.nextFrame();
      return true;
    case "b":
      player.cycle("audio");
      return true;
    case "v":
      player.toggleSubtitles();
      return true;
    case "a":
      player.setAspect(cycleValue(ASPECT_RATIOS, player.aspect));
      return true;
    case "c":
      player.setCrop(cycleValue(CROPS, player.crop));
      return true;
    case "g":
      player.setSubtitleDelay(s.subtitleDelay - 50);
      return true;
    case "h":
      player.setSubtitleDelay(s.subtitleDelay + 50);
      return true;
    case "j":
      player.setAudioDelay(s.audioDelay - 50);
      return true;
    case "k":
      player.setAudioDelay(s.audioDelay + 50);
      return true;
  }

  if (/^[0-9]$/.test(e.key) && s.length > 0 && s.seekable) {
    player.seek((Number(e.key) / 10) * s.length);
    return true;
  }
  return false;
}
