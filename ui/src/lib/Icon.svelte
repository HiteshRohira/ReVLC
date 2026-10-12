<script lang="ts" module>
  // 24×24 icons. Filled ones are marked with `fill`.
  const ICONS = {
    play: { d: "M7 4.5v15l12-7.5z", fill: true },
    pause: { d: "M6.5 4h4v16h-4zM13.5 4h4v16h-4z", fill: true },
    previous: { d: "M18 5v14L8 12zM5.5 5h2v14h-2z", fill: true },
    next: { d: "M6 5v14l10-7zM16.5 5h2v14h-2z", fill: true },
    volume: { d: "M11 5 6 9H3v6h3l5 4zM15.5 8.5a5 5 0 0 1 0 7M18.5 5.5a9 9 0 0 1 0 13" },
    volumeLow: { d: "M11 5 6 9H3v6h3l5 4zM15.5 8.5a5 5 0 0 1 0 7" },
    mute: { d: "M11 5 6 9H3v6h3l5 4zM22 9l-6 6M16 9l6 6" },
    fullscreen: { d: "M4 9V4h5M20 9V4h-5M4 15v5h5M20 15v5h-5" },
    exitFullscreen: { d: "M9 4v5H4M15 4v5h5M9 20v-5H4M15 20v-5h5" },
    playlist: { d: "M4 6h16M4 12h10M4 18h8M16 14v7l5-3.5z" },
    subtitles: { d: "M3 5h18v14H3zM7 13h3M13 13h4M7 16h10" },
    audio: { d: "M3 15v-3a9 9 0 0 1 18 0v3M3 15h4v5H3zM17 15h4v5h-4z" },
    video: { d: "M6 2v14a2 2 0 0 0 2 2h14M2 6h14a2 2 0 0 1 2 2v14" },
    speed: { d: "M12 14l4-4M3.3 19a10 10 0 1 1 17.4 0" },
    plus: { d: "M12 5v14M5 12h14" },
    trash: { d: "M4 7h16M10 11v6M14 11v6M5 7l1 13h12l1-13M9 7V4h6v3" },
    repeat: { d: "M17 2l4 4-4 4M3 11V9a3 3 0 0 1 3-3h15M7 22l-4-4 4-4M21 13v2a3 3 0 0 1-3 3H3" },
    shuffle: { d: "M16 3h5v5M4 20 21 3M21 16v5h-5M15 15l6 6M4 4l5 5" },
    close: { d: "M6 6l12 12M18 6 6 18" },
    link: {
      d: "M10 13a5 5 0 0 0 7.5.5l3-3a5 5 0 0 0-7-7l-1.7 1.7M14 11a5 5 0 0 0-7.5-.5l-3 3a5 5 0 0 0 7 7l1.7-1.7",
    },
    folder: { d: "M3 6a1 1 0 0 1 1-1h5l2 2h9a1 1 0 0 1 1 1v10a1 1 0 0 1-1 1H4a1 1 0 0 1-1-1z" },
    camera: { d: "M4 8h3l2-3h6l2 3h3v11H4zM12 9.5a3.5 3.5 0 1 0 0 7 3.5 3.5 0 0 0 0-7" },
    stop: { d: "M6 6h12v12H6z", fill: true },
    music: { d: "M9 18V5l12-2v13M9 18a3 3 0 1 1-6 0 3 3 0 0 1 6 0zM21 16a3 3 0 1 1-6 0 3 3 0 0 1 6 0z" },
    keyboard: { d: "M3 6h18v12H3zM7 10h.01M11 10h.01M15 10h.01M7 14h10" },
  } satisfies Record<string, { d: string; fill?: boolean }>;

  export type IconName = keyof typeof ICONS;
</script>

<script lang="ts">
  let { name, size = 20 }: { name: IconName; size?: number } = $props();
  const icon = $derived(ICONS[name] as { d: string; fill?: boolean });
</script>

<svg
  width={size}
  height={size}
  viewBox="0 0 24 24"
  fill={icon.fill ? "currentColor" : "none"}
  stroke={icon.fill ? "none" : "currentColor"}
  stroke-width="1.8"
  stroke-linecap="round"
  stroke-linejoin="round"
  aria-hidden="true"
>
  <path d={icon.d} />
</svg>
