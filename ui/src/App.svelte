<script lang="ts">
  import { onMount } from "svelte";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Controls from "./lib/Controls.svelte";
  import Icon from "./lib/Icon.svelte";
  import PlaylistPanel from "./lib/PlaylistPanel.svelte";
  import UrlDialog from "./lib/UrlDialog.svelte";
  import { player, isSubtitle } from "./lib/player.svelte";
  import { handleKey, SHORTCUTS } from "./lib/shortcuts";
  import { ui } from "./lib/ui.svelte";

  const appWindow = getCurrentWindow();
  const IDLE_HIDE_MS = 2500;

  let dropping = $state(false);
  let idle = $state(false);
  let pointerOverControls = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  // Hide the controls and cursor while video plays and the mouse is still.
  const controlsHidden = $derived(
    idle && player.playing && player.state.hasVideo && ui.openMenus === 0 && !pointerOverControls && !ui.playlistOpen,
  );
  // Let the native video view show through only when there is video to show.
  const showVideo = $derived(player.active && player.state.hasVideo);

  $effect(() => {
    const title = player.current?.title;
    appWindow.setTitle(title ? `${title} — revlc` : "revlc");
  });

  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), IDLE_HIDE_MS);
  }

  async function openFiles() {
    const files = await openDialog({ multiple: true, title: "Open media" });
    if (!files) return;
    const list = Array.isArray(files) ? files : [files];
    const subtitles = list.filter(isSubtitle);
    const media = list.filter((f) => !isSubtitle(f));
    if (media.length) await player.open(media);
    if (player.active) subtitles.forEach((s) => player.addSubtitle(s));
  }

  async function toggleFullscreen() {
    await appWindow.setFullscreen(!(await appWindow.isFullscreen()));
  }

  async function exitFullscreen() {
    if (await appWindow.isFullscreen()) await appWindow.setFullscreen(false);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (handleKey(e, { openFiles, toggleFullscreen, exitFullscreen })) {
      e.preventDefault();
      wake();
    }
  }

  onMount(() => {
    player.init();
    wake();

    const unlisteners = [
      getCurrentWebview().onDragDropEvent(({ payload }) => {
        if (payload.type === "enter" || payload.type === "over") dropping = true;
        else if (payload.type === "leave") dropping = false;
        else if (payload.type === "drop") {
          dropping = false;
          const subtitles = payload.paths.filter(isSubtitle);
          const media = payload.paths.filter((p) => !isSubtitle(p));
          if (subtitles.length && player.active) subtitles.forEach((s) => player.addSubtitle(s));
          if (media.length) player.open(media);
        }
      }),
      appWindow.onResized(async () => (ui.fullscreen = await appWindow.isFullscreen())),
    ];
    return () => unlisteners.forEach((p) => p.then((off) => off()));
  });
</script>

<svelte:window onkeydown={onKeyDown} onpointermove={wake} onpointerdown={wake} />

<main class:show-video={showVideo} class:hide-cursor={controlsHidden}>
  <div class="titlebar" data-tauri-drag-region class:hidden={controlsHidden || ui.fullscreen}>
    <span data-tauri-drag-region>{player.current?.title ?? "revlc"}</span>
  </div>

  <div class="stage">
    <!-- Clicks here go to the video area; the native video view sits underneath. -->
    <div class="video-area" role="presentation" ondblclick={toggleFullscreen}>
      {#if player.engineError}
        <div class="center">
          <h1>libVLC not found</h1>
          <p class="error">{player.engineError}</p>
        </div>
      {:else if !player.active}
        <div class="center">
          <div class="logo">revlc</div>
          <p>Drop a file here, or</p>
          <div class="start">
            <button class="btn primary" onclick={openFiles}><Icon name="folder" size={16} /> Open files</button>
            <button class="btn" onclick={() => (ui.urlOpen = true)}><Icon name="link" size={16} /> Open URL</button>
          </div>
          <button class="hint" onclick={() => (ui.helpOpen = true)}><Icon name="keyboard" size={14} /> Keyboard shortcuts <kbd>?</kbd></button>
        </div>
      {:else if !player.state.hasVideo}
        <div class="center audio">
          <Icon name="music" size={64} />
          <h1>{player.current?.title ?? ""}</h1>
          {#if player.state.state === "opening" || player.state.state === "buffering"}<p>Loading…</p>{/if}
        </div>
      {/if}

      <div class="notices" aria-live="polite">
        {#each player.notices as notice (notice.id)}
          <div class="notice {notice.kind}">{notice.text}</div>
        {/each}
      </div>
    </div>

    <div
      class="controls-wrap"
      class:hidden={controlsHidden}
      role="presentation"
      onpointerenter={() => (pointerOverControls = true)}
      onpointerleave={() => (pointerOverControls = false)}
    >
      <Controls onfullscreen={toggleFullscreen} />
    </div>
  </div>

  {#if ui.playlistOpen}
    <PlaylistPanel onadd={openFiles} />
  {/if}

  {#if dropping}
    <div class="drop-overlay"><div>Drop to play · subtitle files are added to the current video</div></div>
  {/if}

  {#if ui.urlOpen}
    <UrlDialog />
  {/if}

  {#if ui.helpOpen}
    <div class="help-backdrop" role="presentation" onpointerdown={() => (ui.helpOpen = false)}>
      <div class="help" role="dialog" aria-label="Keyboard shortcuts">
        <h2>Keyboard shortcuts</h2>
        <dl>
          {#each SHORTCUTS as [keys, action]}
            <dt>{keys}</dt>
            <dd>{action}</dd>
          {/each}
        </dl>
      </div>
    </div>
  {/if}
</main>

<style>
  main {
    position: fixed;
    inset: 0;
    display: flex;
    background: radial-gradient(ellipse at 50% 30%, #1d1d24, var(--bg) 70%);
  }
  main.show-video {
    background: transparent;
  }
  main.hide-cursor,
  main.hide-cursor * {
    cursor: none !important;
  }
  .titlebar {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    height: 38px;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 0 90px;
    font-size: 13px;
    color: var(--muted);
    background: linear-gradient(to bottom, rgb(0 0 0 / 0.55), transparent);
    z-index: 10;
    transition: opacity 0.2s;
  }
  .titlebar span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stage {
    position: relative;
    flex: 1;
    min-width: 0;
  }
  .video-area {
    position: absolute;
    inset: 0;
  }
  .controls-wrap {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    transition: opacity 0.2s;
  }
  .hidden {
    opacity: 0;
    pointer-events: none;
  }
  .center {
    position: absolute;
    inset: 0 0 90px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    text-align: center;
    padding: 0 24px;
  }
  .logo {
    font-size: 44px;
    font-weight: 700;
    letter-spacing: -0.03em;
    background: linear-gradient(135deg, #ff8a3d, #ff4d6d);
    -webkit-background-clip: text;
    background-clip: text;
    color: transparent;
  }
  .center p {
    margin: 0;
    color: var(--muted);
  }
  .center h1 {
    margin: 0;
    font-size: 18px;
    font-weight: 600;
  }
  .error {
    max-width: 520px;
    font-size: 13px;
  }
  .start {
    display: flex;
    gap: 10px;
    margin-top: 6px;
  }
  .hint {
    display: flex;
    align-items: center;
    gap: 6px;
    margin-top: 18px;
    color: var(--muted);
    font-size: 12px;
  }
  .audio {
    color: var(--muted);
  }
  .audio h1 {
    color: var(--text);
    max-width: 80%;
  }
  .notices {
    position: absolute;
    top: 48px;
    right: 20px;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    pointer-events: none;
    z-index: 30;
  }
  .notice {
    max-width: 420px;
    padding: 8px 12px;
    border-radius: 8px;
    background: rgb(0 0 0 / 0.7);
    font-size: 13px;
  }
  .notice.osd {
    font-size: 15px;
    font-weight: 500;
  }
  .notice.error {
    background: rgb(120 20 30 / 0.92);
  }
  .drop-overlay {
    position: fixed;
    inset: 12px;
    display: grid;
    place-items: center;
    border: 2px dashed var(--accent);
    border-radius: 16px;
    background: rgb(0 0 0 / 0.6);
    font-size: 15px;
    z-index: 60;
    pointer-events: none;
  }
  .help-backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.5);
    z-index: 50;
  }
  .help {
    padding: 20px 24px;
    border-radius: 14px;
    background: var(--panel);
    border: 1px solid var(--border);
    max-height: 80vh;
    overflow-y: auto;
  }
  .help h2 {
    margin: 0 0 12px;
    font-size: 16px;
  }
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 6px 24px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    color: var(--text);
    font-variant-numeric: tabular-nums;
    font-weight: 500;
  }
  dd {
    margin: 0;
    color: var(--muted);
  }
</style>
