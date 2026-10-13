/** UI-only state shared between components. */
export const ui = $state({
  openMenus: 0,
  playlistOpen: false,
  /** The open player popover, if any. */
  menu: null as "settings" | "subtitles" | null,
  urlOpen: false,
  helpOpen: false,
  fullscreen: false,
});
