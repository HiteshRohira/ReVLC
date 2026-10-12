<script lang="ts">
  import type { Snippet } from "svelte";

  let {
    selected = false,
    onclick,
    children,
  }: { selected?: boolean; onclick: () => void; children: Snippet } = $props();
</script>

<button class="item" class:selected role="menuitemradio" aria-checked={selected} {onclick}>
  <span class="check" aria-hidden="true">{selected ? "✓" : ""}</span>
  <span class="label">{@render children()}</span>
</button>

<style>
  .item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 7px 10px;
    border-radius: 7px;
    text-align: left;
    color: var(--text);
  }
  .item:hover,
  .item:focus-visible {
    background: var(--hover);
  }
  .check {
    width: 14px;
    color: var(--accent);
    font-size: 13px;
  }
  .label {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .selected {
    color: #fff;
  }
</style>
