<script lang="ts">
  import type { Snippet } from "svelte";
  let { label, help, children }: { label: string; help?: string; children: Snippet } = $props();
</script>

<div class="row">
  <div class="lab">{label}{#if help}<small>{help}</small>{/if}</div>
  <div class="ctl">{@render children()}</div>
</div>

<style>
  .row {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 20px;
    min-height: 44px;
    padding: 8px 14px;
  }
  .lab {
    font-size: 13.5px;
  }
  .lab small {
    display: block;
    margin-top: 2px;
    font-size: 11.5px;
    color: var(--faint);
  }
  .ctl {
    display: flex;
    align-items: center;
    gap: 8px;
    flex-shrink: 0;
  }
</style>
