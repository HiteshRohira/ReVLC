<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { ACTION_BY_ID, effectiveBindings, formatBinding, type Host } from "./actions";
  import { MENUS } from "./menus";
  import { settings } from "./settings.svelte";
  import { ui } from "./ui.svelte";
  import icon from "../../../assets/icon/favicon.svg";

  // The Windows title strip over the home screen: app icon, VLC-style menus
  // and room to drag. The window buttons sit over its right end.
  let { host }: { host: Host } = $props();

  let open = $state<number | null>(null);
  let active = $state(-1);
  let bar: HTMLElement;
  // Alt pressed and released on its own opens the menus, as in Windows.
  let altAlone = false;

  const bindings = $derived(effectiveBindings(settings.value?.shortcuts ?? {}));
  const items = $derived(open === null ? [] : MENUS[open].items);

  function show(i: number | null, first = false) {
    open = i;
    active = first && i !== null ? next(-1, 1, MENUS[i].items) : -1;
  }

  function next(from: number, step: number, list = items) {
    for (let i = 1; i <= list.length; i++) {
      const at = (from + step * i + list.length * i) % list.length;
      if (list[at][0] !== "-") return at;
    }
    return -1;
  }

  function run(id: string) {
    show(null);
    const action = ACTION_BY_ID.get(id);
    if (action) action.run(host);
    else if (id === "settings") invoke("open_settings");
    else invoke("app_menu_item", { id });
  }

  function onKeyDown(e: KeyboardEvent) {
    altAlone = e.key === "Alt" && !e.repeat;
    if (open === null) return;
    // While a menu is open it gets every key.
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.key === "Escape" || e.key === "Alt") show(null);
    else if (e.key === "ArrowDown") active = next(active, 1);
    else if (e.key === "ArrowUp") active = next(active < 0 ? 0 : active, -1);
    else if (e.key === "ArrowRight") show((open + 1) % MENUS.length, true);
    else if (e.key === "ArrowLeft") show((open + MENUS.length - 1) % MENUS.length, true);
    else if ((e.key === "Enter" || e.key === " ") && active >= 0) run(items[active][0]);
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key !== "Alt" || !altAlone) return;
    altAlone = false;
    e.preventDefault();
    if (!ui.dialog) show(open === null ? 0 : null, true);
  }

  function onPointerDown(e: PointerEvent) {
    if (open !== null && !bar.contains(e.target as Node)) show(null);
  }

  // Closes when the app loses focus. Accessibility tools (UI Automation's
  // Invoke, used by Voice Access) blur the window for a moment before they
  // click an item, so only close if focus hasn't come back.
  function onBlur() {
    setTimeout(() => {
      if (!document.hasFocus()) show(null);
    }, 150);
  }
</script>

<svelte:window onkeydowncapture={onKeyDown} onkeyupcapture={onKeyUp} onpointerdowncapture={onPointerDown} onblur={onBlur} />

<header bind:this={bar} data-tauri-drag-region>
  <img src={icon} alt="" data-tauri-drag-region />
  <nav>
    {#each MENUS as m, i}
      <div class="m">
        <button class:open={open === i} onclick={() => show(open === i ? null : i)} onpointerenter={() => open !== null && open !== i && show(i)}>
          {m.label}
        </button>
        {#if open === i}
          <div class="dd" role="menu">
            {#each m.items as [id, label], j}
              {#if id === "-"}<hr />{:else}
                <button role="menuitem" class:on={active === j} onpointerenter={() => (active = j)} onclick={() => run(id)}>
                  {label}<kbd>{id === "settings" ? formatBinding("Cmd+,") : formatBinding(bindings[id] ?? "")}</kbd>
                </button>
              {/if}
            {/each}
          </div>
        {/if}
      </div>
    {/each}
  </nav>
</header>

<style>
  header {
    position: fixed;
    inset: 0 0 auto;
    z-index: 90;
    display: flex;
    align-items: center;
    height: 36px;
    /* The window buttons are 3 × 46px. */
    padding-right: 138px;
    background: var(--sidebar);
    border-bottom: 1px solid var(--border);
    box-sizing: border-box;
  }
  img {
    width: 16px;
    height: 16px;
    margin: 0 10px 0 12px;
  }
  nav {
    display: flex;
    gap: 1px;
  }
  .m {
    position: relative;
  }
  nav > .m > button {
    height: 26px;
    padding: 0 9px;
    border-radius: 6px;
    font-size: 12.5px;
    color: var(--muted);
  }
  nav > .m > button:hover,
  nav > .m > button.open {
    background: var(--hover-strong);
    color: var(--text);
  }
  .dd {
    position: absolute;
    top: calc(100% + 4px);
    left: 0;
    min-width: 260px;
    max-height: calc(100vh - 48px);
    overflow-y: auto;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: #1e1e21;
    box-shadow: 0 18px 50px rgb(0 0 0 / 0.55);
    box-sizing: border-box;
  }
  .dd button {
    display: flex;
    justify-content: flex-start;
    align-items: center;
    gap: 24px;
    width: 100%;
    height: 30px;
    padding: 0 10px;
    border-radius: 7px;
    font-size: 13px;
    white-space: nowrap;
  }
  .dd button.on {
    background: var(--hover);
  }
  kbd {
    margin-left: auto;
    padding: 0;
    border: 0;
    background: none;
    font: inherit;
    font-size: 12px;
    color: var(--faint);
  }
  hr {
    margin: 5px 4px;
    border: 0;
    border-top: 1px solid var(--border);
  }
</style>
