import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Resume = "ask" | "always" | "never";

/** Mirrors src-tauri/src/settings.rs. */
export interface Settings {
  resume: Resume;
  stopAfterEach: boolean;
  pauseWhenMinimized: boolean;
  seekbarChapters: boolean;
  seekbarPreview: boolean;
  autoUpdate: boolean;
  analytics: boolean;

  video: boolean;
  startFullscreen: boolean;
  alwaysOnTop: boolean;
  deinterlace: number;
  deinterlaceMode: string;
  snapshotPath: string;
  snapshotPrefix: string;
  snapshotFormat: string;
  snapshotSequential: boolean;

  audio: boolean;
  audioDevice: string;
  audioLanguage: string;
  timeStretch: boolean;
  replayGain: string;
  spdif: boolean;

  subtitleLanguage: string | null;
  subAutodetect: boolean;
  subAutodetectFuzzy: number;
  subEncoding: string;
  font: string;
  fontSize: number;
  fontColor: number;
  fontOpacity: number;
  bold: boolean;
  outlineThickness: number;
  outlineColor: number;
  backgroundOpacity: number;
  backgroundColor: number;
  shadowOpacity: number;
  subMargin: number;
  showTitle: boolean;
  titleTimeout: number;
  titlePosition: number;

  hardwareDecoding: boolean;
  fileCaching: number;
  networkCaching: number;
  skipLoopFilter: number;

  shortcuts: Record<string, string>;
}

interface Info {
  settings: Settings;
  defaults: Settings;
  systemLanguages: string[];
  audioDevices: [string, string][];
  fonts: string[];
}

class SettingsStore {
  info = $state<Info | null>(null);
  /** Why the last change couldn't be saved, if it couldn't. */
  error = $state<string | null>(null);
  /** Why the settings couldn't be read (e.g. the engine didn't start). */
  loadError = $state<string | null>(null);
  #timer: ReturnType<typeof setTimeout> | undefined;
  #saving: Promise<unknown> = Promise.resolve();

  get value() {
    return this.info?.settings ?? null;
  }

  async load() {
    await listen<Settings>("settings://changed", (e) => {
      if (this.info) this.info.settings = e.payload;
    });
    try {
      this.info = await invoke<Info>("settings_get");
      this.loadError = null;
    } catch (e) {
      this.info = null;
      this.loadError = String(e);
    }
  }

  /**
   * Changes settings. Saving can restart the current video (for options
   * that only apply when it starts), so continuous controls like sliders
   * pass `debounce` to save once the user stops dragging.
   */
  set(patch: Partial<Settings>, debounce = false) {
    if (!this.info) return;
    this.info.settings = { ...this.info.settings, ...patch };
    clearTimeout(this.#timer);
    // One save at a time, in order, each sending the settings as they are by then.
    const save = () =>
      (this.#saving = this.#saving
        .then(() => invoke("settings_set", { settings: $state.snapshot(this.info!.settings) }))
        .then(
          () => (this.error = null),
          (e) => (this.error = String(e)),
        ));
    if (debounce) this.#timer = setTimeout(save, 450);
    else save();
  }

  /** Resets the given keys to their defaults. */
  reset(keys: (keyof Settings)[]) {
    if (!this.info) return;
    const d = this.info.defaults;
    this.set(Object.fromEntries(keys.map((k) => [k, d[k]])) as Partial<Settings>);
  }
}

export const settings = new SettingsStore();
