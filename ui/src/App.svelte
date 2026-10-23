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
  import CaptureDialog from "./lib/CaptureDialog.svelte";
  import DiscDialog from "./lib/DiscDialog.svelte";
  import JumpDialog from "./lib/JumpDialog.svelte";
  import MediaInfoDialog from "./lib/MediaInfoDialog.svelte";
  import UrlDialog from "./lib/UrlDialog.svelte";
  import TitleBar from "./lib/TitleBar.svelte";
  import WindowControls from "./lib/WindowControls.svelte";
  import { isMac } from "./lib/platform";
  import { player, isSubtitle, cleanTitle } from "./lib/player.svelte";
  import { ACTIONS, ACTION_BY_ID, FIXED_SHORTCUTS, REPEATABLE, effectiveBindings, eventBinding, formatBinding, syncMenu, type Host } from "./lib/actions";
  import { formatTime } from "./lib/player.svelte";
  import { Moon } from "@lucide/svelte";
  import { settings } from "./lib/settings.svelte";
  import { theme } from "./lib/theme.svelte";
  import { ui } from "./lib/ui.svelte";
  import { updater } from "./lib/updater.svelte";

  const appWindow = getCurrentWindow();
  const IDLE_HIDE_MS = 2500;

  let dropping = $state(false);
  let idle = $state(false);
  let idleTimer: ReturnType<typeof setTimeout> | undefined;

  const showPlayer = $derived(theme.onboarded && player.active);
  // Hide the controls and cursor while video plays and the mouse is still.
  const controlsHidden = $derived(
    idle && player.playing && player.state.hasVideo && ui.openMenus === 0 && !ui.playlistOpen && !ui.effectsOpen,
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
      // The list on screen stays while this runs; it rescans if the last scan is old.
      player.loadLibrary(false);
      ui.menu = null;
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
    const start = settings.value?.startFullscreen || player.startFullscreen;
    if (!uri || !player.state.hasVideo || !start || fullscreenedFor === uri) return;
    fullscreenedFor = uri;
    appWindow.isFullscreen().then((fs) => {
      if (!fs) appWindow.setFullscreen(true);
    });
  });

  // Keyed on the URI: the item object is rebuilt when, e.g., its title is updated.
  const currentUri = $derived(player.current?.uri ?? null);
  // Only for what's playing: an offer for an item since replaced is stale.
  const resumePrompt = $derived(player.resumeOffer && player.resumeOffer.uri === currentUri ? player.resumeOffer.ms : null);

  // The mouse wheel over the video: volume (VLC's default), seeking, or nothing.
  // A mouse notch is about 40 px; trackpads send many small deltas, which add
  // up to steps. What's left over is dropped once scrolling pauses.
  const WHEEL_STEP = 32;
  let wheelTotal = 0;
  let wheelAt = 0;
  function onWheel(e: WheelEvent) {
    const mode = settings.value?.wheel ?? "volume";
    const target = e.target as HTMLElement;
    // Only over the picture, not over menus, panels or lists that scroll.
    if (mode === "none" || !showPlayer || !target.closest(".stage, .player") || target.closest(".bottom")) return;
    wake();
    if (e.timeStamp - wheelAt > 400) wheelTotal = 0;
    wheelAt = e.timeStamp;
    const lines = e.deltaMode === 1 ? WHEEL_STEP : e.deltaMode === 2 ? 400 : 1;
    wheelTotal += e.deltaY * lines;
    const steps = Math.trunc(wheelTotal / WHEEL_STEP);
    if (steps === 0) return;
    wheelTotal -= steps * WHEEL_STEP;
    // Scrolling up (negative delta) turns it up or jumps forward.
    if (mode === "volume") {
      player.setVolume(player.state.volume - steps * 5);
      player.notify("osd", `Volume ${player.state.volume}%`);
    } else if (player.state.seekable) {
      player.seekBy(-steps * 5_000);
    }
  }

  function wake() {
    idle = false;
    clearTimeout(idleTimer);
    idleTimer = setTimeout(() => (idle = true), IDLE_HIDE_MS);
  }

  async function openFiles() {
    const files = await openDialog({ multiple: true, title: "Open media" });
    if (!files) return;
    const list = Array.isArray(files) ? files : [files];
    // Together, so subtitles go with the video opened along with them.
    await player.open(list);
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
    if (!theme.onboarded || ui.dialog) return;
    const target = e.target as HTMLElement | null;
    if (target?.closest("input, select, textarea")) return;
    const menuKey = MENU_KEYS[e.key];
    const primary = isMac ? e.metaKey : e.ctrlKey;
    if (player.inMenu && menuKey && !primary && !e.shiftKey) {
      // DVD and Blu-ray menus take the arrow keys and Return.
      player.navigate(menuKey);
    } else if (e.key === "Escape") {
      if (ui.menu) ui.menu = null;
      else if (ui.effectsOpen) ui.effectsOpen = false;
      else if (ui.playlistOpen) ui.playlistOpen = false;
      else if (ui.helpOpen) ui.helpOpen = false;
      else exitFullscreen();
    } else if (/^[0-9]$/.test(e.key) && !primary && player.state.length > 0 && player.state.seekable) {
      player.seek((Number(e.key) / 10) * player.state.length);
    } else {
      const binding = eventBinding(e);
      // On macOS, ⌘ shortcuts belong to the menu bar, which runs them itself.
      if (!binding || (isMac && binding.includes("Cmd"))) return;
      const action = ACTIONS.find((a) => bindings[a.id] === binding);
      if (action) {
        // Holding a key repeats it: fine for seeking, not for play/pause.
        if (!e.repeat || REPEATABLE.has(action.id)) action.run(host);
      } else if (binding === "Cmd+,") invoke("open_settings");
      else return;
    }
    e.preventDefault();
    wake();
  }

  const MENU_KEYS: Record<string, "up" | "down" | "left" | "right" | "activate"> = {
    ArrowUp: "up", ArrowDown: "down", ArrowLeft: "left", ArrowRight: "right", Enter: "activate",
  };

  // Minutes left on the sleep timer, refreshed every few seconds.
  let now = $state(Date.now());
  $effect(() => {
    if (player.state.sleepAt === null) return;
    now = Date.now();
    const timer = setInterval(() => (now = Date.now()), 5000);
    return () => clearInterval(timer);
  });
  const sleepLeft = $derived(
    player.state.sleepAt !== null ? `${Math.max(1, Math.ceil((player.state.sleepAt - now) / 60000))} min` : player.state.sleepAfterItem ? "End of item" : null,
  );

  function answerResume(resume: boolean) {
    if (resume && resumePrompt !== null) invoke("resume_at", { ms: resumePrompt });
    player.offerResume(null);
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
    updater.start();
    wake();
    const unlisteners = [
      getCurrentWebview().onDragDropEvent(({ payload }) => {
        if (payload.type === "enter" || payload.type === "over") dropping = true;
        else if (payload.type === "leave") dropping = false;
        else if (payload.type === "drop") {
          dropping = false;
          if (!theme.onboarded) theme.finishOnboarding();
          // Together, so subtitles go with the video dropped along with them.
          const media = payload.paths.some((p) => !isSubtitle(p));
          player.open(payload.paths).then(() => {
            if (media) player.loadLibrary();
          });
        }
      }),
      // The native video view eats mouse moves, so the app forwards them.
      listen("native://pointer", wake),
      listen<string>("menu://action", (e) => {
        ACTION_BY_ID.get(e.payload)?.run(host);
        wake();
      }),
      appWindow.onResized(async () => (ui.fullscreen = await appWindow.isFullscreen())),
    ];
    return () => unlisteners.forEach((p) => p.then((off) => off()));
  });
</script>

<svelte:window onkeydown={onKeyDown} onpointermove={wake} onpointerdown={wake} onwheel={onWheel} />
<svelte:document onvisibilitychange={onVisibility} />

<div class="app" class:video={showPlayer && player.state.hasVideo} class:hide-cursor={controlsHidden} class:fullscreen={ui.fullscreen}>
  {#if !theme.onboarded}
    <Onboarding />
  {:else if showPlayer}
    <div class="stage" role="presentation" ondblclick={(e) => e.target === e.currentTarget && toggleFullscreen()}></div>
    <PlayerView hidden={controlsHidden} onfullscreen={toggleFullscreen} onadd={openFiles} />
  {:else}
    {#if !isMac}<TitleBar {host} />{/if}
    <Home onopen={openFiles} onopenfolder={openFolder} />
  {/if}

  {#if !isMac && !ui.fullscreen}<WindowControls hidden={controlsHidden} />{/if}

  {#if resumePrompt !== null && showPlayer}
    <div class="resume" role="alertdialog" aria-label="Continue where you left off">
      <span>Continue from <b class="tnum">{formatTime(resumePrompt)}</b>?</span>
      <button class="btn pri" onclick={() => answerResume(true)}>Continue</button>
      <button class="btn" onclick={() => answerResume(false)}>Start over</button>
    </div>
  {/if}

  {#if updater.prompt && !controlsHidden}
    <div class="update" role="alertdialog" aria-label="Update ready">
      <span><b>ReVLC {updater.state.version}</b> is ready. Restart to finish updating.</span>
      <button class="btn pri" onclick={updater.restart}>Restart now</button>
      <button class="btn" onclick={updater.later}>Later</button>
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

  {#if sleepLeft && showPlayer && !controlsHidden}
    <button class="sleep tnum" data-tip="Sleep timer (click to turn off)" aria-label="Sleep timer: {sleepLeft}. Turn off" onclick={() => player.setSleepTimer(null)}><Moon size={13} />{sleepLeft}</button>
  {/if}

  {#if ui.dialog === "url"}<UrlDialog />
  {:else if ui.dialog === "jump"}<JumpDialog />
  {:else if ui.dialog === "disc"}<DiscDialog />
  {:else if ui.dialog === "capture"}<CaptureDialog />
  {:else if ui.dialog === "info"}<MediaInfoDialog />{/if}

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
  .resume .btn,
  .update .btn {
    height: 30px;
  }
  .update {
    position: absolute;
    top: 64px;
    right: 16px;
    display: flex;
    align-items: center;
    gap: 10px;
    min-height: 46px;
    padding: 8px 8px 8px 16px;
    border-radius: 12px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 10px 30px rgb(0 0 0 / 0.5);
    font-size: 13.5px;
    z-index: 45;
  }
  .sleep {
    position: absolute;
    top: 6px;
    right: 14px;
    display: inline-flex;
    align-items: center;
    gap: 5px;
    height: 22px;
    padding: 0 9px;
    border-radius: 11px;
    background: rgb(20 20 22 / 0.8);
    color: var(--muted);
    font-size: 12px;
    z-index: 3;
  }
  .sleep:hover {
    color: var(--text);
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
