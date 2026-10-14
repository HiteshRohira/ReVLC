<script lang="ts">
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  // Minimise, maximise and close, drawn by the UI on Windows (the window has no title bar).
  let { hidden = false }: { hidden?: boolean } = $props();

  const win = getCurrentWindow();
  let maximized = $state(false);

  onMount(() => {
    win.isMaximized().then((m) => (maximized = m));
    const off = win.onResized(async () => (maximized = await win.isMaximized()));
    return () => off.then((f) => f());
  });
</script>

<div class="caption" class:hidden>
  <button aria-label="Minimise" title="Minimise" onclick={() => win.minimize()}>
    <svg viewBox="0 0 10 10"><path d="M0 5h10" /></svg>
  </button>
  <button aria-label={maximized ? "Restore" : "Maximise"} title={maximized ? "Restore" : "Maximise"} onclick={() => win.toggleMaximize()}>
    {#if maximized}
      <svg viewBox="0 0 10 10"><path d="M2.5 2.5V.5h7v7h-2M.5 2.5h7v7h-7z" /></svg>
    {:else}
      <svg viewBox="0 0 10 10"><path d="M.5.5h9v9h-9z" /></svg>
    {/if}
  </button>
  <button class="close" aria-label="Close" title="Close" onclick={() => win.close()}>
    <svg viewBox="0 0 10 10"><path d="M.5.5l9 9M9.5.5l-9 9" /></svg>
  </button>
</div>

<style>
  .caption {
    position: fixed;
    top: 0;
    right: 0;
    z-index: 100;
    display: flex;
    transition: opacity 0.25s;
  }
  .caption.hidden {
    opacity: 0;
    pointer-events: none;
  }
  button {
    display: grid;
    place-items: center;
    width: 46px;
    height: 32px;
    border-radius: 0;
    color: var(--text, #fff);
  }
  button:hover {
    background: rgb(255 255 255 / 0.1);
  }
  button:active {
    background: rgb(255 255 255 / 0.06);
  }
  .close:hover {
    background: #c42b1c;
    color: #fff;
  }
  svg {
    width: 10px;
    height: 10px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1;
    shape-rendering: crispEdges;
  }
</style>
