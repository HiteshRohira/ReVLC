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
  import { ACTIONS, ACTION_BY_ID, FIXED_SHORTCUTS, effectiveBindings, eventBinding, formatBinding, syncMenu, type Host } from "./lib/actions";
  import { formatTime } from "./lib/player.svelte";
  import { settings } from "./lib/settings.svelte";
  import { theme } from "./lib/theme.svelte";
  import { ui } from "./lib/ui.svelte";

  const appWindow = getCurrentWindow();
  const IDLE_HIDE_MS = 2500;

  let dropping = $state(false);
  /** A resume point waiting for the user's answer ("Ask" setting). */
  let resumePrompt = $state<number | null>(null);
  let resumeTimer: ReturnType<typeof setTimeout> | undefined;
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

  // Media opened before onboarding finishes (e.g. a first launch from
  // Finder) should play straight away; the accent can be picked later.
  $effect(() => {
    if (player.active && !theme.onboarded) theme.finishOnboarding();
  });

  // Back on the home screen: refresh "continue watching" and close player panels.
  $effect(() => {
    if (!showPlayer) {
      player.loadHistory();
      player.loadLibrary();
      ui.settingsOpen = false;
    }
  });

  const bindings = $derived(effectiveBindings(settings.value?.shortcuts ?? {}));
  const host: Host = { openFiles: () => openFiles(), openFolder: () => openFolder(), toggleFullscreen: () => toggleFullscreen() };

  // Keep the menu bar's shortcuts in step with the user's bindings.
  $effect(() => {
    if (settings.value) syncMenu(bindings);
  });

  $effect(() => {
    if (settings.value) appWindow.setAlwaysOnTop(settings.value.alwaysOnTop);
  });

  // "Start in fullscreen": once per item, when its video appears.
  let fullscreenedFor: string | null = null;
  $effect(() => {
    const uri = currentUri;
    if (!uri || !player.state.hasVideo || !settings.value?.startFullscreen || fullscreenedFor === uri) return;
    fullscreenedFor = uri;
    appWindow.isFullscreen().then((fs) => {
      if (!fs) appWindow.setFullscreen(true);
    });
  });

  // A different item started: drop any stale resume offer. (Keyed on the
  // URI: the item object is rebuilt when, e.g., its title is updated.)
  const currentUri = $derived(player.current?.uri ?? null);
  $effect(() => {
    currentUri;
    resumePrompt = null;
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
    if (!theme.onboarded || ui.urlOpen) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, select, textarea")) return;
    if (e.key === "Escape") {
      if (ui.settingsOpen) ui.settingsOpen = false;
      else if (ui.playlistOpen) ui.playlistOpen = false;
      else if (ui.helpOpen) ui.helpOpen = false;
      else exitFullscreen();
    } else if (/^[0-9]$/.test(e.key) && !e.metaKey && player.state.length > 0 && player.state.seekable) {
      player.seek((Number(e.key) / 10) * player.state.length);
    } else {
      const binding = eventBinding(e);
      // ⌘ shortcuts belong to the menu bar, which runs them itself.
      if (!binding || binding.includes("Cmd")) return;
      const action = ACTIONS.find((a) => bindings[a.id] === binding);
      if (!action) return;
      action.run(host);
    }
    e.preventDefault();
    wake();
  }

  function answerResume(resume: boolean) {
    if (resume && resumePrompt !== null) invoke("resume_at", { ms: resumePrompt });
    resumePrompt = null;
    clearTimeout(resumeTimer);
  }

  function onVisibility() {
    // "Pause when minimised": hidden can also mean covered, so check.
    if (document.hidden && settings.value?.pauseWhenMinimized && player.playing) {
      appWindow.isMinimized().then((min) => {
        if (min) player.togglePause();
      });
    }
  }

  onMount(() => {
    player.init();
    settings.load();
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
      listen<string>("menu://action", (e) => {
        ACTION_BY_ID.get(e.payload)?.run(host);
        wake();
      }),
      listen<number>("player://resume-prompt", (e) => {
        resumePrompt = e.payload;
        clearTimeout(resumeTimer);
        resumeTimer = setTimeout(() => (resumePrompt = null), 12000);
      }),
      appWindow.onResized(async () => (ui.fullscreen = await appWindow.isFullscreen())),
    ];
    return () => unlisteners.forEach((p) => p.then((off) => off()));
  });
</script>

<svelte:window onkeydown={onKeyDown} onpointermove={wake} onpointerdown={wake} />
<svelte:document onvisibilitychange={onVisibility} />

<div class="app" class:video={showPlayer && player.state.hasVideo} class:hide-cursor={controlsHidden} class:fullscreen={ui.fullscreen}>
  {#if !theme.onboarded}
    <Onboarding />
  {:else if showPlayer}
    <div class="stage" role="presentation" ondblclick={(e) => e.target === e.currentTarget && toggleFullscreen()}></div>
    <PlayerView hidden={controlsHidden} onfullscreen={toggleFullscreen} onadd={openFiles} />
  {:else}
    <Home onopen={openFiles} onopenfolder={openFolder} />
  {/if}

  {#if resumePrompt !== null && showPlayer}
    <div class="resume" role="alertdialog" aria-label="Continue where you left off">
      <span>Continue from <b class="tnum">{formatTime(resumePrompt)}</b>?</span>
      <button class="btn pri" onclick={() => answerResume(true)}>Continue</button>
      <button class="btn" onclick={() => answerResume(false)}>Start over</button>
    </div>
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
          {#each ACTIONS.filter((a) => bindings[a.id]) as a (a.id)}<dt>{formatBinding(bindings[a.id])}</dt><dd>{a.label}</dd>{/each}
          {#each FIXED_SHORTCUTS as [k, label]}<dt>{formatBinding(k)}</dt><dd>{label}</dd>{/each}
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
  .resume {
    position: absolute;
    top: 52px;
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 8px 8px 16px;
    border-radius: 12px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.5);
    font-size: 13.5px;
    z-index: 45;
  }
  .resume .btn {
    height: 30px;
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
