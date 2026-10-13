<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { getCurrentWebview } from "@tauri-apps/api/webview";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import Home from "./lib/Home.svelte";
  import Onboarding from "./lib/Onboarding.svelte";
  import PlayerView from "./lib/PlayerView.svelte";
  import UrlDialog from "./lib/UrlDialog.svelte";
  import { player, isSubtitle, cleanTitle } from "./lib/player.svelte";
  import { handleKey, SHORTCUTS } from "./lib/shortcuts";
  import { theme } from "./lib/theme.svelte";
  import { ui } from "./lib/ui.svelte";

  const appWindow = getCurrentWindow();
  const IDLE_HIDE_MS = 2500;

  let dropping = $state(false);
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  const showPlayer = $derived(theme.onboarded && player.active);
  // Hide the controls and cursor while video plays and the mouse is still.
  const controlsHidden = $derived(
    idle && player.playing && player.state.hasVideo && ui.openMenus === 0 && !ui.playlistOpen,
  );

  $effect(() => {
    if (controlsHidden) invoke("hide_cursor");
  });

  $effect(() => {
    const title = player.current?.title;
    appWindow.setTitle(showPlayer && title ? `${cleanTitle(title)} — ReVLC` : "ReVLC");
  });

  // Back on the home screen: refresh "continue watching" and close player panels.
  $effect(() => {
    if (!showPlayer) {
      player.loadHistory();
      player.loadLibrary();
      ui.settingsOpen = false;
    }
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
    const media = list.filter((f) => !isSubtitle(f));
    if (media.length) await player.open(media);
    if (player.active) list.filter(isSubtitle).forEach((s) => player.addSubtitle(s));
  }

  /** Plays a folder and adds it to the library. */
  async function openFolder() {
    const dir = await openDialog({ directory: true, title: "Add a folder" });
    if (typeof dir !== "string") return;
    await player.open([dir]);
    await player.loadLibrary();
  }

  async function toggleFullscreen() {
    await appWindow.setFullscreen(!(await appWindow.isFullscreen()));
  }

  async function exitFullscreen() {
    if (await appWindow.isFullscreen()) await appWindow.setFullscreen(false);
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!theme.onboarded) return;
    if (handleKey(e, { openFiles, openFolder, toggleFullscreen, exitFullscreen })) {
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
          if (!theme.onboarded) theme.finishOnboarding();
          const media = payload.paths.filter((p) => !isSubtitle(p));
          if (player.active) payload.paths.filter(isSubtitle).forEach((s) => player.addSubtitle(s));
          if (media.length) player.open(media).then(() => player.loadLibrary());
        }
      }),
      // The native video view eats mouse moves, so the app forwards them.
      listen("native://pointer", wake),
      appWindow.onResized(async () => (ui.fullscreen = await appWindow.isFullscreen())),
    ];
    return () => unlisteners.forEach((p) => p.then((off) => off()));
  });
</script>

<svelte:window onkeydown={onKeyDown} onpointermove={wake} onpointerdown={wake} />

<div class="app" class:video={showPlayer && player.state.hasVideo} class:hide-cursor={controlsHidden} class:fullscreen={ui.fullscreen}>
  {#if !theme.onboarded}
    <Onboarding />
  {:else if showPlayer}
    <div class="stage" role="presentation" ondblclick={(e) => e.target === e.currentTarget && toggleFullscreen()}></div>
    <PlayerView hidden={controlsHidden} onfullscreen={toggleFullscreen} onadd={openFiles} />
  {:else}
    <Home onopen={openFiles} onopenfolder={openFolder} />
  {/if}

  <div class="notices" aria-live="polite">
    {#each player.notices as notice (notice.id)}
      <div class="notice {notice.kind}">{notice.text}</div>
    {/each}
  </div>

  {#if dropping}
    <div class="drop"><div>Drop to play{#if player.active} · subtitle files are added to this video{/if}</div></div>
  {/if}

  {#if ui.urlOpen}<UrlDialog />{/if}

  {#if ui.helpOpen}
    <div class="backdrop" role="presentation" onpointerdown={() => (ui.helpOpen = false)}>
      <div class="help" role="dialog" aria-label="Keyboard shortcuts">
        <h2>Keyboard shortcuts</h2>
        <dl>
          {#each SHORTCUTS as [keys, action]}<dt>{keys}</dt><dd>{action}</dd>{/each}
        </dl>
      </div>
    </div>
  {/if}
</div>

<style>
  .app {
    position: fixed;
    inset: 0;
    background: var(--bg);
  }
  .app.video {
    background: transparent;
  }
  .hide-cursor,
  .hide-cursor :global(*) {
    cursor: none !important;
  }
  .stage {
    position: absolute;
    inset: 0;
  }
  .notices {
    position: absolute;
    top: 52px;
    right: 20px;
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: 8px;
    pointer-events: none;
    z-index: 40;
  }
  .notice {
    max-width: 420px;
    padding: 8px 13px;
    border-radius: 9px;
    background: rgb(20 20 22 / 0.88);
    font-size: 13px;
  }
  .notice.osd {
    font-size: 14px;
    font-weight: 600;
  }
  .notice.error {
    background: rgb(130 24 34 / 0.95);
  }
  .drop {
    position: fixed;
    inset: 12px;
    display: grid;
    place-items: center;
    border: 2px dashed var(--acc);
    border-radius: 18px;
    background: rgb(0 0 0 / 0.65);
    font-size: 15px;
    font-weight: 600;
    z-index: 60;
    pointer-events: none;
  }
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.55);
    z-index: 50;
  }
  .help {
    padding: 22px 26px;
    border-radius: 16px;
    background: var(--panel);
    border: 1px solid var(--border);
    max-height: 80vh;
    overflow-y: auto;
  }
  .help h2 {
    margin: 0 0 14px;
    font-size: 17px;
  }
  dl {
    display: grid;
    grid-template-columns: auto auto;
    gap: 7px 28px;
    margin: 0;
    font-size: 13px;
  }
  dt {
    font-weight: 600;
  }
  dd {
    margin: 0;
    color: var(--muted);
  }
</style>
