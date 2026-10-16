<script lang="ts">
  import { onMount, type Snippet } from "svelte";
  let { label, help, children }: { label: string; help?: string; children: Snippet } = $props();

  const id = `row-${Math.random().toString(36).slice(2, 8)}`;
  let ctl: HTMLElement;

  // Screen readers name the row's switch, dropdown or field after its label,
  // and read the help text as its description.
  onMount(() => {
    for (const el of ctl.querySelectorAll("input, [role=switch], [role=combobox]")) {
      if (el.hasAttribute("aria-label") || el.hasAttribute("aria-labelledby")) continue;
      el.setAttribute("aria-labelledby", id);
      if (help) el.setAttribute("aria-describedby", `${id}-help`);
    }
  });
</script>

<div class="row">
  <div class="lab"><span {id}>{label}</span>{#if help}<small id="{id}-help">{help}</small>{/if}</div>
  <div class="ctl" bind:this={ctl}>{@render children()}</div>
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
