<script lang="ts">
  import { Check, Download, RefreshCw, RotateCw } from "@lucide/svelte";
  import { updater } from "./updater.svelte";

  // One button for every update state: check, download, progress, restart.
  const R = 13;
  const C = 2 * Math.PI * R;

  const status = $derived(updater.state.status);
  const busy = $derived(status === "checking" || status === "downloading");
  // The spin finishes its turn rather than snapping back when a check ends.
  let spinning = $state(false);
  $effect(() => {
    if (status === "checking") spinning = true;
  });
  const progress = $derived(updater.state.progress);
</script>

<button
  class="upd"
  class:active={status === "available" || status === "downloading" || status === "ready"}
  aria-disabled={busy || undefined}
  aria-label={updater.tip}
  data-tip={updater.tip}
  onclick={() => {
    if (status === "idle" || status === "error") spinning = true;
    updater.act();
  }}
>
  {#if status === "available"}
    <span class="glyph"><Download size={17} /><span class="dot"></span></span>
  {:else if status === "downloading"}
    <svg class="ring" class:indeterminate={progress === null} viewBox="0 0 30 30" aria-hidden="true">
      <circle cx="15" cy="15" r={R} class="track" />
      <circle cx="15" cy="15" r={R} class="bar" stroke-dasharray={C} stroke-dashoffset={C * (1 - (progress ?? 0.25))} />
    </svg>
    <Download size={17} />
  {:else if status === "ready"}
    <span class="glyph"><RotateCw size={17} /><span class="badge"><Check size={8} strokeWidth={3.5} /></span></span>
  {:else}
    <span class="glyph" class:spin={spinning} onanimationiteration={() => (spinning = status === "checking")}>
      <RefreshCw size={17} />
    </span>
    {#if status === "error"}<span class="dot warn"></span>{/if}
  {/if}
</button>

<style>
  .upd {
    position: relative;
    display: grid;
    place-items: center;
    flex: none;
    width: 32px;
    height: 32px;
    border-radius: 50%;
    color: var(--muted);
    transition:
      background 0.15s,
      color 0.15s;
  }
  .upd:hover:not([aria-disabled]) {
    background: var(--hover);
    color: var(--text);
  }
  .upd.active {
    background: var(--hover);
    color: var(--text);
  }
  .upd.active:hover:not([aria-disabled]) {
    background: var(--hover-strong);
  }
  .upd[aria-disabled] {
    cursor: default;
  }
  .glyph {
    position: relative;
    display: grid;
    place-items: center;
  }
  .dot {
    position: absolute;
    top: -2px;
    right: -2px;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    box-shadow: 0 0 0 2px var(--sidebar);
  }
  .dot.warn {
    top: 7px;
    right: 7px;
    background: #ff9f0a;
  }
  .badge {
    position: absolute;
    right: -4px;
    bottom: -4px;
    display: grid;
    place-items: center;
    width: 11px;
    height: 11px;
    border-radius: 50%;
    background: var(--text);
    color: var(--sidebar);
    box-shadow: 0 0 0 2px var(--sidebar);
  }
  .spin {
    animation: spin 0.9s cubic-bezier(0.45, 0, 0.55, 1) infinite;
  }
  .ring {
    position: absolute;
    inset: 1px;
    width: 30px;
    height: 30px;
    transform: rotate(-90deg);
    pointer-events: none;
  }
  .ring circle {
    fill: none;
    stroke-width: 1.5;
  }
  .track {
    stroke: color-mix(in srgb, currentColor 22%, transparent);
  }
  .bar {
    stroke: currentColor;
    stroke-linecap: round;
    transition: stroke-dashoffset 0.3s ease-out;
  }
  /* Size unknown: a quarter arc goes round instead. */
  .ring.indeterminate {
    animation: ring 1.1s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  @keyframes ring {
    from {
      transform: rotate(-90deg);
    }
    to {
      transform: rotate(270deg);
    }
  }
  @media (prefers-reduced-motion: reduce) {
    .spin,
    .ring.indeterminate {
      animation: none;
    }
    .bar {
      transition: none;
    }
  }
</style>
