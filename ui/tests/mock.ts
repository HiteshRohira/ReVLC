// A stand-in for ReVLC's Rust backend, for the browser tests in tests/ui.
// It answers the commands the UI sends with fixed scenarios, records every
// call, and lets tests push events the way the backend does.
//
// The scenario comes from the page URL: ?scenario=home|playing|engine-error
// |settings-error, plus ?view=settings for the Settings window. Tests reach
// it through window.__mock.
import { mockIPC, mockWindows } from "@tauri-apps/api/mocks";
import { emit } from "@tauri-apps/api/event";

type Json = Record<string, unknown>;

const params = new URLSearchParams(location.search);
const scenario = params.get("scenario") ?? "home";
const view = params.get("view");

const VIDEO = "/media/Big Buck Bunny.mp4";
const SECOND = "/media/Sintel.mkv";

const settingsDefaults: Json = {
  resume: "ask", stopAfterEach: false, pauseWhenMinimized: false, seekbarChapters: true, seekbarPreview: true, wheel: "volume",
  autoUpdate: false, analytics: false,
  video: true, startFullscreen: false, alwaysOnTop: false, deinterlace: -1, deinterlaceMode: "auto",
  snapshotPath: "", snapshotPrefix: "vlcsnap-", snapshotFormat: "png", snapshotSequential: false,
  audio: true, audioDevice: "", audioLanguage: "", timeStretch: true, replayGain: "none", spdif: false,
  subtitleLanguage: null, subAutodetect: true, subAutodetectFuzzy: 3, subEncoding: "", font: "Arial", fontSize: 0,
  fontColor: 0xffffff, fontOpacity: 255, bold: false, outlineThickness: 4, outlineColor: 0, backgroundOpacity: 0,
  backgroundColor: 0, shadowOpacity: 128, subMargin: 0, showTitle: false, titleTimeout: 5000, titlePosition: 4,
  hardwareDecoding: true, fileCaching: 1000, networkCaching: 1000, skipLoopFilter: 0, shortcuts: {},
};

const playing = scenario === "playing";
const state: Json = {
  state: playing ? "playing" : "idle", time: playing ? 12_000 : 0, length: playing ? 596_000 : 0, seekable: playing,
  hasVideo: playing, rate: 1, volume: 100, muted: false, buffering: 100, audioDelay: 0, subtitleDelay: 0,
  title: -1, chapter: playing ? 0 : -1, loopA: null, loopB: null, sleepAt: null, sleepAfterItem: false,
};
const tracks: Json = {
  audio: [{ id: -1, name: "Disable" }, { id: 1, name: "Track 1 - [English]" }, { id: 2, name: "Track 2 - [French]" }],
  audioCurrent: 1,
  video: [{ id: -1, name: "Disable" }, { id: 0, name: "Track 1" }],
  videoCurrent: 0,
  subtitles: [{ id: -1, name: "Disable" }, { id: 3, name: "Track 1 - [English]" }],
  subtitleCurrent: -1,
  teletext: false,
  teletextOn: false,
};
const playlist: Json = {
  items: [{ uri: VIDEO, title: "Big Buck Bunny.mp4" }, { uri: SECOND, title: "Sintel.mkv" }],
  current: playing ? 0 : null,
  repeat: "off",
  shuffle: false,
};
const effects: Json = {
  equalizer: { enabled: false, preset: 0, preamp: 12, bands: Array(10).fill(0) },
  stereo: "stereo",
  adjust: { enabled: false, brightness: 1, contrast: 1, hue: 0, saturation: 1, gamma: 1 },
  logo: { enabled: false, file: "", position: 6, opacity: 255 },
  marquee: { enabled: false, text: "", position: 5, size: 0, color: 0xffffff, opacity: 255, timeout: 0 },
  zoom: 0,
  teletextTransparent: true,
  transform: "none",
  filters: {},
};
let settings: Json = { ...settingsDefaults };
const history = [
  { uri: VIDEO, title: "Big Buck Bunny.mp4", position: 120_000, length: 596_000, updated: 1_790_000_000, finished: false, thumb: null },
];
const library = { folders: ["/media"], items: [] as Json[] };

/** Every command the UI sent, with its arguments. */
const calls: { cmd: string; args: unknown }[] = [];
/** Commands the mock doesn't know: a test fails on any. */
const unknown: string[] = [];
/** Commands made to fail, by name, with the error text. */
const failing: Record<string, string> = {};

async function push(event: string, payload: unknown) {
  await emit(event, payload);
}

function setState(patch: Json) {
  Object.assign(state, patch);
  void push("player://state", { ...state });
}

const handlers: Record<string, (args: Json) => unknown> = {
  snapshot: () =>
    scenario === "engine-error"
      ? { state: null, tracks: null, playlist: null, chapters: [], titles: [], bookmarks: [], effects, startFullscreen: false,
          engine: { Err: "Couldn't load C:\\ReVLC\\libvlccore.dll: LoadLibraryExW failed" }, early: { notices: [], resume: null } }
      : { state: { ...state }, tracks: playing ? tracks : null, playlist: playing ? playlist : null,
          chapters: playing ? [{ start: 0, duration: 300_000, name: "Opening" }, { start: 300_000, duration: 296_000, name: "The chase" }] : [],
          titles: [], bookmarks: [], effects, startFullscreen: false, engine: { Ok: "3.0.24 Vetinari" }, early: { notices: [], resume: null } },
  settings_get: () => {
    if (scenario === "engine-error" || scenario === "settings-error") throw "the playback engine couldn't start";
    return { settings, defaults: settingsDefaults, systemLanguages: ["en"], audioDevices: [["", "System default"]], fonts: ["Arial", "Helvetica Neue"] };
  },
  settings_set: (a) => {
    settings = a.settings as Json;
  },
  history_list: () => history,
  history_remove: (a) => {
    const i = history.findIndex((h) => h.uri === a.uri);
    if (i >= 0) history.splice(i, 1);
  },
  library_list: () => library,
  library_rescan: () => library,
  filter_specs: () => ({
    video: [
      { id: "invert", label: "Invert colours", module: "invert", params: [] },
      { id: "sepia", label: "Sepia", module: "sepia", params: [{ key: "intensity", label: "Intensity", min: 0, max: 255, step: 1, default: 120, unit: "", int: true }] },
    ],
    audio: [{ id: "pitch", label: "Pitch", module: "scaletempo_pitch", params: [{ key: "pitch-shift", label: "Shift", min: -12, max: 12, step: 0.5, default: 0, unit: " semitones", int: false }] }],
  }),
  equalizer_info: () => ({ presets: ["Flat", "Classical", "Club", "Dance", "Rock"], bands: [60, 170, 310, 600, 1000, 3000, 6000, 12000, 14000, 16000] }),
  media_info: () => ({
    location: VIDEO,
    meta: [["Title", "Big Buck Bunny"]],
    tracks: [{ kind: "video", id: 0, codec: "h264", codecName: "H.264", language: null, description: null, bitrate: 0, channels: 0, rate: 0,
               width: 1920, height: 1080, fps: 24, sar: null, encoding: null }],
  }),
  media_stats: () => ({ readBytes: 1_000_000, inputBitrate: 2.5, demuxReadBytes: 900_000, demuxBitrate: 2.4, demuxCorrupted: 0, demuxDiscontinuity: 0,
                        decodedVideo: 300, decodedAudio: 500, displayedPictures: 298, lostPictures: 0, playedAbuffers: 500, lostAbuffers: 0 }),
  update_state: () => ({ status: "idle", version: "", progress: null, byUser: false }),
  disc_list: () => [],
  capture_devices: () => ({ video: [], audio: [] }),
  scrub_prepare: () => null,
  toggle_pause: () => setState({ state: state.state === "playing" ? "paused" : "playing" }),
  seek_by: (a) => setState({ time: Math.max(0, (state.time as number) + (a.deltaMs as number)) }),
  seek: (a) => setState({ time: a.ms }),
  set_volume: (a) => setState({ volume: a.volume }),
  set_muted: (a) => setState({ muted: a.muted }),
  set_rate: (a) => setState({ rate: a.rate }),
  stop: () => setState({ state: "stopped", time: 0, length: 0, hasVideo: false }),
};

mockWindows(view === "settings" ? "settings" : "main", "main", "settings");
mockIPC(
  (cmd, args) => {
    calls.push({ cmd, args });
    if (cmd in failing) throw failing[cmd];
    if (cmd in handlers) return handlers[cmd]((args ?? {}) as Json);
    // Window, webview and dialog plugin calls: nothing to do here.
    if (cmd.startsWith("plugin:")) return null;
    // Player commands the UI sends without needing an answer.
    if (/^(set_|open|next|previous|play_index|playlist_|bookmark_|add_subtitle|navigate|take_snapshot|resume_at|app_menu_item|hide_cursor|open_settings|update_|scrub_|library_|history_)/.test(cmd)) return null;
    unknown.push(cmd);
    return null;
  },
  { shouldMockEvents: true },
);

declare global {
  interface Window {
    __mock: {
      calls: typeof calls;
      unknown: string[];
      failing: Record<string, string>;
      emit: typeof push;
      setState: typeof setState;
      callsTo(cmd: string): unknown[];
    };
  }
}

window.__mock = {
  calls,
  unknown,
  failing,
  emit: push,
  setState,
  callsTo: (cmd) => calls.filter((c) => c.cmd === cmd).map((c) => c.args),
};
