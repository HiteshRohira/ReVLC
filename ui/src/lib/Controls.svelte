<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Icon from "./Icon.svelte";
  import Menu from "./Menu.svelte";
  import MenuItem from "./MenuItem.svelte";
  import MenuHeading from "./MenuHeading.svelte";
  import Stepper from "./Stepper.svelte";
  import { player, formatTime, ASPECT_RATIOS, CROPS, SPEEDS, SUBTITLE_EXTENSIONS } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let { onfullscreen }: { onfullscreen: () => void } = $props();

  const s = $derived(player.state);
  let dragging = $state<number | null>(null);
  let hover = $state<{ x: number; ms: number } | null>(null);
  let showRemaining = $state(false);

  const position = $derived(dragging ?? s.time);
  const progress = $derived(s.length > 0 ? Math.min(100, (position / s.length) * 100) : 0);
  const canSeek = $derived(player.active && s.seekable && s.length > 0);
  const volumeIcon = $derived(s.muted || s.volume === 0 ? "mute" : s.volume < 50 ? "volumeLow" : "volume");

  function onSeekHover(e: PointerEvent) {
    const el = e.currentTarget as HTMLElement;
    const rect = el.getBoundingClientRect();
    const ratio = Math.min(1, Math.max(0, (e.clientX - rect.left) / rect.width));
    hover = { x: e.clientX - rect.left, ms: ratio * s.length };
  }

  async function loadSubtitle() {
    const file = await openDialog({
      multiple: false,
      title: "Load subtitle file",
      filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }],
    });
    if (typeof file === "string") player.addSubtitle(file);
  }
</script>

<div class="controls">
  <div class="seek" class:disabled={!canSeek}>
    <span class="time">{formatTime(position)}</span>
    <div class="track" role="presentation" onpointermove={onSeekHover} onpointerleave={() => (hover = null)}>
      <input
        type="range"
        min="0"
        max={Math.max(1, s.length)}
        step="1000"
        value={position}
        disabled={!canSeek}
        style="--progress: {progress}%"
        aria-label="Seek"
        aria-valuetext={formatTime(position)}
        oninput={(e) => (dragging = Number(e.currentTarget.value))}
        onchange={(e) => {
          player.seek(Number(e.currentTarget.value));
          dragging = null;
        }}
      />
      {#if hover && canSeek}
        <span class="tooltip" style="left: {hover.x}px">{formatTime(hover.ms)}</span>
      {/if}
    </div>
    <button class="time end" title="Toggle remaining time" onclick={() => (showRemaining = !showRemaining)}>
      {#if s.length > 0}
        {showRemaining ? `−${formatTime(s.length - position)}` : formatTime(s.length)}
      {:else if player.active}
        Live
      {:else}
        0:00
      {/if}
    </button>
  </div>

  <div class="row">
    <div class="group">
      <button class="icon-btn" title="Previous (P)" aria-label="Previous" onclick={player.previous}>
        <Icon name="previous" />
      </button>
      <button
        class="icon-btn play"
        title={player.playing ? "Pause (Space)" : "Play (Space)"}
        aria-label={player.playing ? "Pause" : "Play"}
        onclick={player.togglePause}
      >
        <Icon name={player.playing ? "pause" : "play"} size={26} />
      </button>
      <button class="icon-btn" title="Next (N)" aria-label="Next" onclick={player.next}>
        <Icon name="next" />
      </button>

      <div class="volume">
        <button
          class="icon-btn"
          title="Mute (M)"
          aria-label={s.muted ? "Unmute" : "Mute"}
          onclick={() => player.setMuted(!s.muted)}
        >
          <Icon name={volumeIcon} />
        </button>
        <input
          type="range"
          min="0"
          max="150"
          value={s.muted ? 0 : s.volume}
          style="--progress: {((s.muted ? 0 : s.volume) / 150) * 100}%"
          aria-label="Volume"
          aria-valuetext="{s.volume}%"
          oninput={(e) => player.setVolume(Number(e.currentTarget.value))}
        />
        <span class="volume-label">{s.muted ? "Muted" : `${s.volume}%`}</span>
      </div>
    </div>

    <div class="title" title={player.current?.title}>{player.current?.title ?? ""}</div>

    <div class="group">
      <Menu icon="speed" label="Playback speed">
        <MenuHeading>Speed</MenuHeading>
        {#each SPEEDS as speed}
          <MenuItem selected={Math.abs(s.rate - speed) < 0.01} onclick={() => player.setRate(speed)}>
            {speed === 1 ? "Normal" : `${speed}×`}
          </MenuItem>
        {/each}
      </Menu>

      <Menu icon="audio" label="Audio">
        <MenuHeading>Audio track</MenuHeading>
        {#each player.tracks.audio as track (track.id)}
          <MenuItem selected={track.id === player.tracks.audioCurrent} onclick={() => player.setAudioTrack(track.id)}>
            {track.name}
          </MenuItem>
        {:else}
          <div class="empty">No audio tracks</div>
        {/each}
        <Stepper
          label="Audio delay"
          value={s.audioDelay}
          step={50}
          unit="ms"
          onchange={(v) => player.setAudioDelay(v)}
        />
      </Menu>

      <Menu icon="subtitles" label="Subtitles">
        <MenuHeading>Subtitles</MenuHeading>
        {#each player.tracks.subtitles as track (track.id)}
          <MenuItem
            selected={track.id === player.tracks.subtitleCurrent}
            onclick={() => player.setSubtitleTrack(track.id)}
          >
            {track.name}
          </MenuItem>
        {:else}
          <div class="empty">No subtitles</div>
        {/each}
        <MenuItem onclick={loadSubtitle}>Load subtitle file…</MenuItem>
        <Stepper
          label="Subtitle delay"
          value={s.subtitleDelay}
          step={50}
          unit="ms"
          onchange={(v) => player.setSubtitleDelay(v)}
        />
      </Menu>

      <Menu icon="video" label="Video" align="end">
        <MenuHeading>Aspect ratio</MenuHeading>
        {#each ASPECT_RATIOS as ratio}
          <MenuItem selected={player.aspect === ratio} onclick={() => player.setAspect(ratio)}>
            {ratio ?? "Default"}
          </MenuItem>
        {/each}
        <MenuHeading>Crop</MenuHeading>
        {#each CROPS as crop}
          <MenuItem selected={player.crop === crop} onclick={() => player.setCrop(crop)}>
            {crop ?? "None"}
          </MenuItem>
        {/each}
        <MenuHeading>Other</MenuHeading>
        <MenuItem selected={player.deinterlace} onclick={() => player.setDeinterlace(!player.deinterlace)}>
          Deinterlace
        </MenuItem>
        <MenuItem onclick={player.snapshot}>Take snapshot</MenuItem>
      </Menu>

      <button
        class="icon-btn"
        class:active={ui.playlistOpen}
        title="Playlist (⌘L)"
        aria-label="Playlist"
        onclick={() => (ui.playlistOpen = !ui.playlistOpen)}
      >
        <Icon name="playlist" />
      </button>
      <button
        class="icon-btn"
        title="Fullscreen (F)"
        aria-label={ui.fullscreen ? "Exit fullscreen" : "Fullscreen"}
        onclick={onfullscreen}
      >
        <Icon name={ui.fullscreen ? "exitFullscreen" : "fullscreen"} />
      </button>
    </div>
  </div>
</div>

<style>
  .controls {
    padding: 28px 18px 12px;
    background: linear-gradient(to bottom, transparent, rgb(0 0 0 / 0.55) 40%, rgb(0 0 0 / 0.8));
  }
  .seek {
    display: flex;
    align-items: center;
    gap: 12px;
  }
  .time {
    min-width: 52px;
    font-size: 12px;
    font-variant-numeric: tabular-nums;
    color: var(--text);
  }
  .time.end {
    text-align: right;
  }
  .track {
    position: relative;
    flex: 1;
    display: flex;
    align-items: center;
  }
  .track input {
    width: 100%;
  }
  .tooltip {
    position: absolute;
    bottom: 20px;
    transform: translateX(-50%);
    padding: 3px 7px;
    border-radius: 6px;
    background: var(--panel);
    font-size: 11px;
    font-variant-numeric: tabular-nums;
    pointer-events: none;
  }
  .disabled {
    opacity: 0.6;
  }
  .row {
    display: flex;
    align-items: center;
    gap: 12px;
    margin-top: 6px;
  }
  .group {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  .play {
    width: 44px;
    height: 44px;
  }
  .title {
    flex: 1;
    min-width: 0;
    text-align: center;
    font-size: 13px;
    color: var(--muted);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .volume {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-left: 6px;
  }
  .volume input {
    width: 90px;
  }
  .volume-label {
    width: 44px;
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
  }
  .empty {
    padding: 7px 10px 7px 32px;
    color: var(--muted);
  }
  @media (max-width: 640px) {
    .title,
    .volume-label {
      display: none;
    }
  }
</style>
