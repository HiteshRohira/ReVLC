<script lang="ts">
  import type { Snippet } from "svelte";

  // A sidebar section's page: its title, its buttons, and what it lists.
  // The lists share their looks, so those are here too.
  let { title, actions, children }: { title: string; actions?: Snippet; children: Snippet } = $props();
</script>

<div class="page">
  <div class="top" data-tauri-drag-region>
    <h1 data-tauri-drag-region>{title}</h1>
    <span class="sp" data-tauri-drag-region></span>
    {@render actions?.()}
  </div>
  {@render children()}
</div>

<style>
  .page {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-width: 0;
    min-height: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 64px;
    flex: none;
    padding: 0 24px 0 28px;
  }
  h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
  }
  .sp {
    flex: 1;
    align-self: stretch;
  }
  .page :global(.scroll) {
    flex: 1;
    overflow-y: auto;
    padding: 4px 28px 24px;
  }
  .page :global(.scroll h2) {
    margin: 6px 0 12px;
    font-size: 18px;
    font-weight: 700;
  }
  /* A small heading over a group of rows. */
  .page :global(.label) {
    margin: 18px 10px 6px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--faint);
  }
  .page :global(.label:first-child) {
    margin-top: 4px;
  }
  .page :global(.btn.small) {
    height: 28px;
    padding: 0 10px;
    font-size: 12px;
  }
  .page :global(.btn.danger) {
    background: rgb(190 40 52);
    color: #fff;
  }

  /* Rows: an icon, a name, a detail, and buttons that show on hover. */
  .page :global(.rows) {
    display: flex;
    flex-direction: column;
    gap: 1px;
    margin: 0 0 8px;
    padding: 0;
    list-style: none;
  }
  .page :global(.row) {
    position: relative;
    display: flex;
    align-items: center;
    gap: 4px;
    border-radius: 9px;
  }
  .page :global(.row:hover),
  .page :global(.row:focus-within) {
    background: var(--hover);
  }
  .page :global(.row .main) {
    flex: 1;
    display: flex;
    justify-content: flex-start;
    align-items: center;
    gap: 12px;
    min-width: 0;
    padding: 9px 10px;
    border-radius: 9px;
    text-align: left;
  }
  .page :global(.row .main > svg),
  .page :global(.row .num) {
    flex: none;
    color: var(--muted);
  }
  .page :global(.row .num) {
    width: 22px;
    text-align: right;
    font-size: 12.5px;
  }
  .page :global(.row .t) {
    flex: 0 1 auto;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
  }
  .page :global(.row .d) {
    flex: 1 1 0;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 12.5px;
    color: var(--muted);
  }
  .page :global(.row .end) {
    flex: none;
    margin-left: auto;
    font-size: 12.5px;
    color: var(--muted);
  }
  .page :global(.row .act) {
    flex: none;
    width: 30px;
    height: 30px;
    margin-right: 4px;
    border-radius: 7px;
    color: var(--muted);
    opacity: 0;
  }
  .page :global(.row:hover .act),
  .page :global(.row .act:focus-visible),
  .page :global(.row .act.on) {
    opacity: 1;
  }
  .page :global(.row .act:hover) {
    background: var(--hover-strong);
    color: var(--text);
  }
  .page :global(.row .act.on) {
    color: var(--acc);
  }

  /* Nothing here yet: what this is for, and what to do. */
  .page :global(.blank) {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    margin: 0 24px 24px;
    border: 1.5px dashed rgb(255 255 255 / 0.14);
    border-radius: 18px;
    color: var(--faint);
    text-align: center;
  }
  .page :global(.blank h2) {
    margin: 8px 0 0;
    font-size: 20px;
    color: var(--text);
  }
  .page :global(.blank p) {
    margin: 0;
    max-width: 440px;
    color: var(--muted);
  }
  .page :global(.blank .btns) {
    display: flex;
    gap: 10px;
    margin-top: 14px;
  }
  .page :global(.note) {
    margin: 10px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .page :global(.note.error) {
    color: #ff8f96;
  }

  /* The way back from a playlist, an album or a folder. */
  .page :global(.crumbs) {
    display: flex;
    align-items: center;
    flex-wrap: wrap;
    gap: 2px;
    margin: 0 0 12px -8px;
    font-size: 13px;
    color: var(--muted);
  }
  .page :global(.crumbs button) {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    height: 26px;
    padding: 0 8px;
    border-radius: 7px;
    color: var(--muted);
  }
  .page :global(.crumbs button:hover) {
    background: var(--hover);
    color: var(--text);
  }
  .page :global(.crumbs b) {
    padding: 0 8px;
    font-weight: 600;
    color: var(--text);
  }

  /* A row of choices, one of them on. */
  .page :global(.chips) {
    display: flex;
    flex-wrap: wrap;
    gap: 6px;
    margin: 0 0 14px;
  }
  .page :global(.chip) {
    height: 28px;
    padding: 0 12px;
    border-radius: 14px;
    background: var(--hover);
    font-size: 12.5px;
    color: var(--muted);
  }
  .page :global(.chip:hover) {
    background: var(--hover-strong);
    color: var(--text);
  }
  .page :global(.chip.on) {
    background: var(--acc);
    color: var(--on-acc);
    font-weight: 600;
  }
</style>
