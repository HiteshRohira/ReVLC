import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type PlayState = "idle" | "opening" | "buffering" | "playing" | "paused" | "stopped" | "ended" | "error";
export type Repeat = "off" | "all" | "one";

export interface PlayerState {
  state: PlayState;
  time: number;
  length: number;
  seekable: boolean;
  hasVideo: boolean;
  rate: number;
  volume: number;
  muted: boolean;
  /** Input cache fill, 0–100; below 100 while loading or after a seek. */
  buffering: number;
  audioDelay: number;
  subtitleDelay: number;
}

export interface Track {
  id: number;
  name: string;
}

export interface Tracks {
  audio: Track[];
  audioCurrent: number;
  subtitles: Track[];
  subtitleCurrent: number;
}

export interface Chapter {
  start: number;
  duration: number;
  name: string;
}

export interface HistoryEntry {
  uri: string;
  title: string;
  position: number;
  length: number;
  updated: number;
  finished: boolean;
  thumb: string | null;
}

export interface LibraryItem {
  uri: string;
  title: string;
  root: string;
  folder: string;
  modified: number;
  thumb: string | null;
}

export interface Library {
  folders: string[];
  items: LibraryItem[];
}

export interface Item {
  uri: string;
  title: string;
}

export interface Playlist {
  items: Item[];
  current: number | null;
  repeat: Repeat;
  shuffle: boolean;
}

interface Snapshot {
  state: PlayerState | null;
  tracks: Tracks | null;
  playlist: Playlist | null;
  chapters: Chapter[];
  engine: { Ok: string } | { Err: string };
}

export interface Notice {
  id: number;
  kind: "info" | "error" | "osd";
  text: string;
}

export const SUBTITLE_EXTENSIONS = ["srt", "ass", "ssa", "vtt", "sub", "idx", "sup", "smi", "ttml"];
export const ASPECT_RATIOS = [null, "16:9", "4:3", "1:1", "16:10", "2.21:1", "2.35:1", "2.39:1", "5:4"];
export const CROPS = [null, "16:9", "4:3", "1:1", "16:10", "2.21:1", "2.35:1", "2.39:1", "5:4"];
export const SPEEDS = [0.25, 0.5, 0.75, 1, 1.25, 1.5, 1.75, 2, 3, 4];

const ACTIVE: PlayState[] = ["opening", "buffering", "playing", "paused"];

class Player {
  state = $state<PlayerState>({
    state: "idle",
    time: 0,
    length: 0,
    seekable: false,
    hasVideo: false,
    rate: 1,
    volume: 100,
    muted: false,
    buffering: 100,
    audioDelay: 0,
    subtitleDelay: 0,
  });
  tracks = $state<Tracks>({ audio: [], audioCurrent: -1, subtitles: [], subtitleCurrent: -1 });
  playlist = $state<Playlist>({ items: [], current: null, repeat: "off", shuffle: false });
  chapters = $state<Chapter[]>([]);
  history = $state<HistoryEntry[]>([]);
  library = $state<Library>({ folders: [], items: [] });
  /** Bumped when a thumbnail file changes, to bust the web view's image cache. */
  thumbVersion = $state(0);
  engine = $state("");
  engineError = $state<string | null>(null);
  notices = $state<Notice[]>([]);
  aspect = $state<string | null>(null);
  crop = $state<string | null>(null);
  deinterlace = $state(false);

  active = $derived(ACTIVE.includes(this.state.state));
  playing = $derived(this.state.state === "playing" || this.state.state === "buffering" || this.state.state === "opening");
  /** Waiting for data: opening, or refilling the cache after a seek. */
  loading = $derived(
    this.state.state === "opening" ||
      this.state.state === "buffering" ||
      (this.state.state === "playing" && this.state.buffering < 100),
  );
  current = $derived(this.playlist.current != null ? (this.playlist.items[this.playlist.current] ?? null) : null);
  chapter = $derived(chapterAt(this.chapters, this.state.time));

  #noticeId = 0;

  async init() {
    await Promise.all([
      listen<PlayerState>("player://state", (e) => (this.state = e.payload)),
      listen<Tracks>("player://tracks", (e) => (this.tracks = e.payload)),
      listen<Playlist>("player://playlist", (e) => (this.playlist = e.payload)),
      listen<Chapter[]>("player://chapters", (e) => (this.chapters = e.payload)),
      listen<string>("library://thumb", () => {
        this.thumbVersion++;
        this.#refreshLibrarySoon();
      }),
      listen<{ kind: "info" | "error"; text: string }>("player://notice", (e) =>
        this.notify(e.payload.kind, e.payload.text),
      ),
    ]);
    const snap = await invoke<Snapshot>("snapshot");
    if ("Err" in snap.engine) {
      this.engineError = snap.engine.Err;
      return;
    }
    this.engine = snap.engine.Ok;
    if (snap.state) this.state = snap.state;
    if (snap.tracks) this.tracks = snap.tracks;
    if (snap.playlist) this.playlist = snap.playlist;
    this.chapters = snap.chapters;
    await Promise.all([this.loadHistory(), this.loadLibrary()]);
  }

  async loadLibrary() {
    try {
      this.library = await invoke<Library>("library_list");
    } catch {
      this.library = { folders: [], items: [] };
    }
  }

  #refreshTimer: ReturnType<typeof setTimeout> | undefined;
  // Thumbnails arrive one by one; refresh the lists at most every couple of seconds.
  #refreshLibrarySoon() {
    this.#refreshTimer ??= setTimeout(() => {
      this.#refreshTimer = undefined;
      this.loadLibrary();
      this.loadHistory();
    }, 1500);
  }

  async addFolder(path: string) {
    try {
      this.library = await invoke<Library>("library_add", { path });
    } catch (e) {
      this.notify("error", String(e));
    }
  }

  async removeFolder(path: string) {
    this.library = await invoke<Library>("library_remove", { path });
  }

  async loadHistory() {
    try {
      this.history = await invoke<HistoryEntry[]>("history_list");
    } catch {
      this.history = [];
    }
  }

  async forget(uri: string) {
    await this.#call("history_remove", { uri });
    await this.loadHistory();
  }

  notify(kind: Notice["kind"], text: string) {
    const id = ++this.#noticeId;
    // Only one OSD message at a time; it replaces the previous one.
    this.notices = [...this.notices.filter((n) => kind !== "osd" || n.kind !== "osd"), { id, kind, text }];
    setTimeout(() => (this.notices = this.notices.filter((n) => n.id !== id)), kind === "error" ? 6000 : 1500);
  }

  async #call(cmd: string, args?: Record<string, unknown>) {
    try {
      await invoke(cmd, args);
    } catch (e) {
      this.notify("error", String(e));
    }
  }

  open = (uris: string[], play = true) => this.#call("open", { uris, play });
  playIndex = (index: number) => this.#call("play_index", { index });
  togglePause = () => this.#call("toggle_pause");
  stop = async () => {
    await this.#call("stop");
    await this.loadHistory();
  };
  next = () => this.#call("next");
  previous = () => this.#call("previous");
  nextFrame = () => this.#call("next_frame");

  seek = (ms: number) => this.#call("seek", { ms: Math.round(ms) });
  seekBy = (deltaMs: number) => this.#call("seek_by", { deltaMs: Math.round(deltaMs) });

  setVolume(volume: number) {
    volume = Math.max(0, Math.min(200, Math.round(volume)));
    this.state.volume = volume;
    return this.#call("set_volume", { volume });
  }
  setMuted = (muted: boolean) => this.#call("set_muted", { muted });

  setRate(rate: number) {
    rate = Math.max(0.25, Math.min(4, Math.round(rate * 100) / 100));
    this.notify("osd", `Speed ${rate}×`);
    return this.#call("set_rate", { rate });
  }

  setAudioTrack = (id: number) => this.#call("set_audio_track", { id });
  setSubtitleTrack = (id: number) => this.#call("set_subtitle_track", { id });
  addSubtitle = (uri: string) => this.#call("add_subtitle", { uri });

  setAudioDelay(ms: number) {
    this.notify("osd", `Audio delay ${ms} ms`);
    return this.#call("set_audio_delay", { ms });
  }
  setSubtitleDelay(ms: number) {
    this.notify("osd", `Subtitle delay ${ms} ms`);
    return this.#call("set_subtitle_delay", { ms });
  }

  setAspect(ratio: string | null) {
    this.aspect = ratio;
    this.notify("osd", `Aspect ratio: ${ratio ?? "Default"}`);
    return this.#call("set_aspect_ratio", { ratio });
  }
  setCrop(geometry: string | null) {
    this.crop = geometry;
    this.notify("osd", `Crop: ${geometry ?? "None"}`);
    return this.#call("set_crop", { geometry });
  }
  setDeinterlace(on: boolean) {
    this.deinterlace = on;
    this.notify("osd", `Deinterlace ${on ? "on" : "off"}`);
    return this.#call("set_deinterlace", { mode: on ? "yadif" : null });
  }

  snapshot = () => this.#call("take_snapshot");

  remove = (index: number) => this.#call("playlist_remove", { index });
  move = (from: number, to: number) => this.#call("playlist_move", { from, to });
  clear = () => this.#call("playlist_clear");
  setRepeat = (repeat: Repeat) => this.#call("set_repeat", { repeat });
  setShuffle = (shuffle: boolean) => this.#call("set_shuffle", { shuffle });

  #lastSubtitle = -1;

  /** Turns subtitles off, or back on to the last (or first) track. */
  toggleSubtitles() {
    const { subtitles, subtitleCurrent } = this.tracks;
    if (subtitleCurrent !== -1) {
      this.#lastSubtitle = subtitleCurrent;
      this.notify("osd", "Subtitles off");
      return this.setSubtitleTrack(-1);
    }
    const next = subtitles.find((t) => t.id === this.#lastSubtitle) ?? subtitles.find((t) => t.id !== -1);
    if (!next) return this.notify("osd", "No subtitles");
    this.notify("osd", `Subtitles: ${next.name}`);
    return this.setSubtitleTrack(next.id);
  }

  /** Selects the next entry in a track list, wrapping around. */
  cycle(kind: "audio" | "subtitles") {
    const list = kind === "audio" ? this.tracks.audio : this.tracks.subtitles;
    if (list.length === 0) return;
    const current = kind === "audio" ? this.tracks.audioCurrent : this.tracks.subtitleCurrent;
    const next = list[(list.findIndex((t) => t.id === current) + 1) % list.length];
    this.notify("osd", `${kind === "audio" ? "Audio" : "Subtitles"}: ${next.name}`);
    return kind === "audio" ? this.setAudioTrack(next.id) : this.setSubtitleTrack(next.id);
  }
}

export const player = new Player();

export function isSubtitle(path: string) {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  return SUBTITLE_EXTENSIONS.includes(ext);
}

const MEDIA_EXT = /\.(mp4|m4v|mkv|webm|mov|avi|ts|m2ts|mts|mpg|mpeg|wmv|flv|ogv|3gp|vob|mp3|m4a|aac|flac|wav|ogg|opus|wma|aiff)$/i;

/** A display title: file names lose their extension. */
export function cleanTitle(title: string) {
  return title.replace(MEDIA_EXT, "");
}

/** libVLC calls the "no track" entry "Disable". */
export function trackName(name: string) {
  return name === "Disable" ? "Off" : name;
}

/** URL for a thumbnail file in the app cache. */
export function thumbUrl(path: string | null, version = 0) {
  return path ? `${convertFileSrc(path)}?v=${version}` : null;
}

/** True for placeholder chapter names like "Chapter 01". */
export function genericChapter(name: string) {
  return /^(chapter|chap\.?)\s*\d+$/i.test(name.trim());
}

export function chapterAt(chapters: Chapter[], ms: number): Chapter | null {
  let found: Chapter | null = null;
  for (const c of chapters) if (c.start <= ms) found = c;
  return found;
}

/** Whether a history entry was left part-way through (mirrors the backend's rule). */
export function resumable(e: HistoryEntry) {
  return !e.finished && e.position > 15_000 && (e.length === 0 || e.position < e.length - 15_000);
}

export function formatTime(ms: number) {
  const total = Math.max(0, Math.floor(ms / 1000));
  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;
  const pad = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${pad(m)}:${pad(s)}` : `${m}:${pad(s)}`;
}
