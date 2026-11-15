export type HomeView = "home" | "videos" | "music" | "playlists" | "network" | "radio";

/** UI-only state shared between components. */
export const ui = $state({
  openMenus: 0,
  playlistOpen: false,
  /** The open player popover, if any. */
  menu: null as "settings" | "subtitles" | null,
  /** The open modal dialog, if any. */
  dialog: null as "url" | "jump" | "disc" | "capture" | "info" | "playlist" | "server" | null,
  /** The media information dialog's tab. */
  infoTab: "general" as "general" | "codec" | "stats",
  effectsOpen: false,
  helpOpen: false,
  fullscreen: false,
  /** The sidebar section on show. Kept here so it's still there after playback. */
  homeView: "home" as HomeView,
});
