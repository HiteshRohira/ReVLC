<script lang="ts">
  import { untrack } from "svelte";
  import { Captions, ChevronLeft, ListVideo, Maximize, Minimize, Music2, Settings, SkipBack, SkipForward, Volume1, Volume2, VolumeX } from "@lucide/svelte";
  import EffectsPanel from "./EffectsPanel.svelte";
  import QueuePanel from "./QueuePanel.svelte";
  import SeekBar from "./SeekBar.svelte";
  import SettingsMenu from "./SettingsMenu.svelte";
  import Transport from "./Transport.svelte";
  import { player, formatTime, genericChapter, cleanTitle, thumbUrl } from "./player.svelte";
  import { ui } from "./ui.svelte";
  import { modKey } from "./platform";

  let { hidden, onfullscreen, onadd }: { hidden: boolean; onfullscreen: () => void; onadd: () => void } = $props();

  const s = $derived(player.state);
  const VolumeIcon = $derived(s.muted || s.volume === 0 ? VolumeX : s.volume < 50 ? Volume1 : Volume2);
  // Placeholder names like "Chapter 01" add nothing next to the title.
  const chapterName = $derived(
    player.chapters.length > 1 && player.chapter && !genericChapter(player.chapter.name) ? player.chapter.name : null,
  );
  const subtitlesOn = $derived(player.tracks.subtitleCurrent !== -1 && player.tracks.subtitles.length > 0);
  let menusEl = $state<HTMLDivElement>();
  // Shown only if loading lasts a moment, so local seeks don't flash it.
  let spinner = $state(false);
  // libVLC reports "playing" before the first frame, and while a stream
  // refills after a seek, so a clock that stops moving counts as buffering.
  let stalled = $state(false);
  // Primitives, so the effect below reruns only when they actually change.
  const time = $derived(s.time);
  const playState = $derived(s.state);

  $effect(() => {
    time;
    stalled = false;
    if (playState !== "playing") return;
    const timer = setTimeout(() => (stalled = true), 700);
    return () => clearTimeout(timer);
  });

  $effect(() => {
    if (!player.loading && !stalled) return;
    const timer = setTimeout(() => (spinner = true), 150);
    return () => {
      clearTimeout(timer);
      spinner = false;
    };
  });

  // The count is written, not depended on: reading it here would make the
  // effect rerun on its own write, which can loop (it froze the UI on Windows).
  $effect(() => {
    if (!ui.menu) return;
    untrack(() => ui.openMenus++);
    return () => untrack(() => ui.openMenus--);
  });

  function onWindowPointer(e: PointerEvent) {
    if (ui.menu && menusEl && !menusEl.contains(e.target as Node)) ui.menu = null;
  }

  const toggleMenu = (menu: "settings" | "subtitles") => (ui.menu = ui.menu === menu ? null : menu);
</script>

<svelte:window onpointerdown={onWindowPointer} />

{#if spinner}
  <div class="loading" role="status" aria-label="Buffering"><div class="ring"></div></div>
{/if}

<!-- Covers the video while the controls show, so it takes the double-click too. -->
<div class="player" class:hidden role="presentation" ondblclick={(e) => e.target === e.currentTarget && onfullscreen()}>
  {#if !s.hasVideo}
    <div class="audio">
      {#if !spinner}
        <Music2 size={56} strokeWidth={1.5} />
        <h1>{cleanTitle(player.current?.title ?? "")}</h1>
      {/if}
    </div>
  {/if}

  {#if player.scrubShown}
    <img class="scrubbed" src={thumbUrl(player.scrubShown, player.scrubVersion)} alt="" draggable="false" />
  {/if}

  <div class="top" data-tauri-drag-region>
    <div class="bar" data-tauri-drag-region>
      <button class="back" data-tip="Back to home" aria-label="Back to home" onclick={player.stop}><ChevronLeft size={18} /></button>
      <div class="title" data-tauri-drag-region>
        {cleanTitle(player.current?.title ?? "")}{#if chapterName}<span>&nbsp;·&nbsp;{chapterName}</span>{/if}
      </div>
    </div>
  </div>

  <div class="bottom">
    <SeekBar />
    <div class="row">
      <div class="grp">
        <button class="icon" data-tip="Previous (P)" aria-label="Previous" onclick={player.previous}><SkipBack size={19} /></button>
        <button class="icon play" data-tip="{player.playing ? 'Pause' : 'Play'} (Space)" aria-label={player.playing ? "Pause" : "Play"} onclick={player.togglePause}>
          <Transport playing={player.playing} size={26} />
        </button>
        <button class="icon" data-tip="Next (N)" aria-label="Next" onclick={player.next}><SkipForward size={19} /></button>
        <div class="volume">
          <button class="icon" data-tip="Mute (M)" aria-label={s.muted ? "Unmute" : "Mute"} onclick={() => player.setMuted(!s.muted)}><VolumeIcon size={20} /></button>
          <input type="range" min="0" max="150" value={s.muted ? 0 : s.volume} style="--progress: {((s.muted ? 0 : s.volume) / 150) * 100}%"
            aria-label="Volume" oninput={(e) => player.setVolume(Number(e.currentTarget.value))} />
        </div>
        <button class="time tnum" data-tip="Jump to time ({modKey}T)" disabled={!s.seekable} onclick={() => (ui.dialog = "jump")}>
          {formatTime(s.time)}&nbsp;<span>/ {s.length > 0 ? formatTime(s.length) : "Live"}</span>
        </button>
      </div>
      <div class="grp">
        <div class="menu-wrap" bind:this={menusEl}>
          <button class="icon cc" class:on={subtitlesOn} class:active={ui.menu === "subtitles"} data-tip="Subtitles" aria-label="Subtitles"
            aria-expanded={ui.menu === "subtitles"} onclick={() => toggleMenu("subtitles")}>
            <Captions size={21} />
          </button>
          <button class="icon" class:active={ui.menu === "settings"} data-tip="Settings" aria-label="Settings" aria-expanded={ui.menu === "settings"} onclick={() => toggleMenu("settings")}>
            <Settings size={20} />
          </button>
          {#if ui.menu}
            {#key ui.menu}<div class="pop"><SettingsMenu kind={ui.menu} onclose={() => (ui.menu = null)} /></div>{/key}
          {/if}
        </div>
        <button class="icon" class:active={ui.playlistOpen} data-tip="Up next ({modKey}L)" aria-label="Up next" onclick={() => (ui.playlistOpen = !ui.playlistOpen)}>
          <ListVideo size={21} />
        </button>
        <button class="icon" data-tip="Fullscreen (F)" aria-label={ui.fullscreen ? "Exit fullscreen" : "Fullscreen"} onclick={onfullscreen}>
          {#if ui.fullscreen}<Minimize size={19} />{:else}<Maximize size={19} />{/if}
        </button>
      </div>
    </div>
  </div>
</div>

{#if ui.playlistOpen}<QueuePanel {onadd} />{/if}
{#if ui.effectsOpen}<EffectsPanel />{/if}

<style>
  .player {
    position: absolute;
    inset: 0;
    transition: opacity 0.25s;
  }
  .player.hidden {
    opacity: 0;
    pointer-events: none;
  }
  .top {
    position: absolute;
    inset: 0 0 auto;
    z-index: 2;
    height: 72px;
    padding: 0 16px 0 76px;
    background: linear-gradient(rgb(0 0 0 / 0.6), transparent);
  }
  :global([data-platform="windows"]) .top {
    padding: 0 150px 0 12px;
  }
  :global(.fullscreen) .top {
    padding-left: 12px;
  }
  /* Same height as the title bar, so the button and title line up with the window controls. */
  .bar {
    display: flex;
    align-items: center;
    gap: 4px;
    height: 32px;
  }
  :global([data-platform="windows"]) .bar {
    height: 36px;
  }
  .back {
    width: 26px;
    height: 26px;
    border-radius: 7px;
    color: #fff;
  }
  .back:hover {
    background: rgb(255 255 255 / 0.12);
  }
  .title {
    font-size: 13.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .title span {
    font-weight: 400;
    color: var(--muted);
  }
  .bottom {
    position: absolute;
    z-index: 2;
    inset: auto 0 0;
    padding: 40px 16px 8px;
    background: linear-gradient(transparent, rgb(0 0 0 / 0.5) 40%, rgb(0 0 0 / 0.78));
  }
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 42px;
  }
  .grp {
    display: flex;
    align-items: center;
    gap: 4px;
  }
  .icon {
    width: 36px;
    height: 36px;
    border-radius: 8px;
    color: #fff;
  }
  .icon:hover {
    background: rgb(255 255 255 / 0.1);
  }
  .icon.active {
    color: var(--acc);
  }
  .play {
    width: 40px;
  }
  .cc {
    position: relative;
  }
  .cc.on::after {
    content: "";
    position: absolute;
    left: 9px;
    right: 9px;
    bottom: 3px;
    height: 2px;
    border-radius: 1px;
    background: var(--acc);
  }
  .volume {
    display: flex;
    align-items: center;
    margin-left: 6px;
  }
  .volume input {
    width: 0;
    opacity: 0;
    transition: width 0.18s, opacity 0.18s, margin 0.18s;
  }
  .volume:hover input,
  .volume:focus-within input {
    width: 72px;
    opacity: 1;
    margin: 0 8px 0 2px;
  }
  .time {
    display: inline-flex;
    margin-left: 4px;
    padding: 4px 6px;
    border-radius: 6px;
    font-size: 13px;
  }
  .time:disabled {
    opacity: 1;
  }
  .time:hover:not(:disabled) {
    background: rgb(255 255 255 / 0.1);
  }
  .time span {
    color: var(--muted);
  }
  .menu-wrap {
    position: relative;
    display: flex;
    gap: 4px;
  }
  .pop {
    position: absolute;
    right: -88px;
    bottom: 48px;
  }
  /* The scrubbed-to frame over the video, scaled up from a small preview. */
  .scrubbed {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    object-fit: contain;
    background: #000;
    pointer-events: none;
  }
  .loading {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(24 24 27 / 0.55);
    pointer-events: none;
    animation: fade-in 0.2s;
    /* Above the audio-only backdrop, below the controls. */
    z-index: 1;
  }
  .ring {
    width: 44px;
    height: 44px;
    border-radius: 50%;
    border: 3.5px solid rgb(255 255 255 / 0.2);
    border-top-color: #fff;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes fade-in {
    from {
      opacity: 0;
    }
  }
  .audio {
    position: absolute;
    inset: 0 0 100px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 12px;
    color: var(--muted);
    background: radial-gradient(ellipse at 50% 40%, #1d1d21, #0d0d0f 70%);
  }
  .audio h1 {
    margin: 0;
    max-width: 70%;
    text-align: center;
    font-size: 20px;
    color: var(--text);
  }
</style>
