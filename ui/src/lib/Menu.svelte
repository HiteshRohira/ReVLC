<script lang="ts">
  import type { Snippet } from "svelte";
  import Icon, { type IconName } from "./Icon.svelte";
  import { ui } from "./ui.svelte";

  let {
    icon,
    label,
    disabled = false,
    align = "center",
    children,
  }: { icon: IconName; label: string; disabled?: boolean; align?: "center" | "end"; children: Snippet } =
    $props();

  let open = $state(false);
  let root: HTMLDivElement;

  $effect(() => {
    if (!open) return;
    ui.openMenus++;
    return () => ui.openMenus--;
  });

  function onPointerDown(e: PointerEvent) {
    if (open && !root.contains(e.target as Node)) open = false;
  }
  function onKeyDown(e: KeyboardEvent) {
    if (open && e.key === "Escape") {
      open = false;
      e.stopPropagation();
    }
  }
</script>

<svelte:window onpointerdown={onPointerDown} onkeydowncapture={onKeyDown} />

<div class="menu" bind:this={root}>
  <button
    class="icon-btn"
    class:active={open}
    title={label}
    aria-label={label}
    aria-haspopup="menu"
    aria-expanded={open}
    {disabled}
    onclick={() => (open = !open)}
  >
    <Icon name={icon} />
  </button>
  {#if open}
    <div class="popover {align}" role="menu" aria-label={label}>
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .menu {
    position: relative;
  }
  .popover {
    position: absolute;
    bottom: calc(100% + 10px);
    min-width: 220px;
    max-height: min(60vh, 420px);
    overflow-y: auto;
    padding: 6px;
    border-radius: 12px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 12px 40px rgb(0 0 0 / 0.5);
    z-index: 20;
  }
  .popover.center {
    left: 50%;
    transform: translateX(-50%);
  }
  .popover.end {
    right: 0;
  }
</style>
