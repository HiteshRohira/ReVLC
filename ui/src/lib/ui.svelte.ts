/** UI-only state shared between components. */
export const ui = $state({
  openMenus: 0,
  playlistOpen: false,
  /** The open player popover, if any. */
  menu: null as "settings" | "subtitles" | null,
  /** The open modal dialog, if any. */
  dialog: null as "url" | "jump" | "disc" | "capture" | null,
  helpOpen: false,
  fullscreen: false,
});
