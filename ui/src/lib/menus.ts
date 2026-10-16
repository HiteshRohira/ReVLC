/**
 * The menus the title strip shows on Windows, laid out like VLC's. Ids are
 * action ids (see actions.ts) or app items the backend runs ("quit",
 * "checkUpdates", "github"); "-" is a separator. Keep the player menus in
 * step with src-tauri/src/menu.rs, which builds the macOS menu bar.
 */
export interface MenuDef {
  label: string;
  items: [id: string, label: string][];
}

const SEP: [string, string] = ["-", ""];

export const MENUS: MenuDef[] = [
  {
    label: "Media",
    items: [
      ["open", "Open File…"],
      ["openFolder", "Open Folder…"],
      ["openDisc", "Open Disc…"],
      ["openUrl", "Open Network Stream…"],
      ["openCapture", "Open Capture Device…"],
      SEP,
      ["savePlaylist", "Save Playlist…"],
      SEP,
      ["quit", "Quit"],
    ],
  },
  {
    label: "Playback",
    items: [
      ["playPause", "Play / Pause"],
      ["stop", "Stop"],
      ["next", "Next"],
      ["previous", "Previous"],
      SEP,
      ["seekForward", "Jump Forward 5 s"],
      ["seekBack", "Jump Back 5 s"],
      ["seekForwardLong", "Jump Forward 30 s"],
      ["seekBackLong", "Jump Back 30 s"],
      ["nextFrame", "Next Frame"],
      ["jumpToTime", "Jump to Time…"],
      ["abLoop", "A–B Loop"],
      SEP,
      ["nextChapter", "Next Chapter"],
      ["previousChapter", "Previous Chapter"],
      ["nextTitle", "Next Title"],
      ["previousTitle", "Previous Title"],
      ["discMenu", "Disc Menu"],
      SEP,
      ["addBookmark", "Add Bookmark"],
      ["sleepTimer", "Sleep Timer"],
      SEP,
      ["faster", "Faster"],
      ["slower", "Slower"],
      ["normalSpeed", "Normal Speed"],
    ],
  },
  {
    label: "Audio",
    items: [
      ["volumeUp", "Increase Volume"],
      ["volumeDown", "Decrease Volume"],
      ["mute", "Mute"],
      SEP,
      ["cycleAudio", "Next Audio Track"],
      ["audioDelayUp", "Increase Audio Delay"],
      ["audioDelayDown", "Decrease Audio Delay"],
    ],
  },
  {
    label: "Video",
    items: [
      ["fullscreen", "Toggle Fullscreen"],
      ["snapshot", "Take Snapshot"],
      SEP,
      ["cycleAspect", "Next Aspect Ratio"],
      ["cycleCrop", "Next Crop"],
      ["deinterlace", "Toggle Deinterlace"],
    ],
  },
  {
    label: "Subtitles",
    items: [
      ["toggleSubtitles", "Subtitles On / Off"],
      ["addSubtitle", "Add Subtitle File…"],
      SEP,
      ["subtitleDelayUp", "Increase Subtitle Delay"],
      ["subtitleDelayDown", "Decrease Subtitle Delay"],
    ],
  },
  {
    label: "Tools",
    items: [
      ["queue", "Up Next"],
      SEP,
      ["settings", "Settings…"],
    ],
  },
  {
    label: "Help",
    items: [
      ["help", "Keyboard Shortcuts"],
      SEP,
      ["checkUpdates", "Check for Updates…"],
      ["github", "ReVLC on GitHub"],
    ],
  },
];
