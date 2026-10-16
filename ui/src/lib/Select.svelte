<script lang="ts" generics="T extends string | number">
  import { Check, ChevronDown } from "@lucide/svelte";
  import { tick } from "svelte";

  // A dropdown drawn like the title strip's menus. A native <select> opens a
  // list the webview draws itself, in light system colours on Windows.
  let {
    value = $bindable(),
    options,
    onchange,
    disabled = false,
    id,
    label,
    class: cls = "",
  }: {
    value: T;
    options: [T, string][];
    onchange?: (value: T) => void;
    disabled?: boolean;
    id?: string;
    label?: string;
    class?: string;
  } = $props();

  const ROW = 30;
  const PAD = 6;
  const uid = `sel-${Math.random().toString(36).slice(2, 8)}`;

  let open = $state(false);
  let active = $state(-1);
  let pos = $state("");
  let button: HTMLButtonElement;
  let list = $state<HTMLElement>();
  // Typed letters jump to the first matching entry, as in a native list.
  let typed = "";
  let typedAt = 0;

  const current = $derived(options.findIndex(([v]) => v === value));
  const shown = $derived(current >= 0 ? options[current][1] : String(value ?? ""));

  async function show() {
    if (disabled || !options.length) return;
    const r = button.getBoundingClientRect();
    const height = Math.min(options.length * ROW + PAD * 2 + 2, 320);
    const below = innerHeight - r.bottom - 8;
    // Opens upwards when there isn't room below and there is more above.
    const up = below < height && r.top - 8 > below;
    const max = Math.max(ROW * 3, (up ? r.top : innerHeight - r.bottom) - 12);
    const place = (left: number) =>
      `left: ${left}px; min-width: ${r.width}px; max-height: ${max}px; ${up ? `bottom: ${innerHeight - r.top + 4}px` : `top: ${r.bottom + 4}px`}`;
    pos = place(r.left);
    active = Math.max(0, current);
    open = true;
    await tick();
    // Lines up with the button; a list wider than the room to its right
    // ends at the button's right edge instead.
    const width = list?.offsetWidth ?? r.width;
    if (r.left + width > innerWidth - 8) pos = place(Math.max(8, Math.min(r.right, innerWidth - 8) - width));
    list?.children[active]?.scrollIntoView({ block: "nearest" });
  }

  function hide() {
    open = false;
  }

  function pick(i: number) {
    hide();
    button.focus();
    const [v] = options[i];
    if (v === value) return;
    value = v;
    onchange?.(v);
  }

  function move(to: number) {
    active = Math.max(0, Math.min(options.length - 1, to));
    list?.children[active]?.scrollIntoView({ block: "nearest" });
  }

  function onKeyDown(e: KeyboardEvent) {
    if (!open) {
      if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
        e.preventDefault();
        show();
      }
      return;
    }
    // While open, the list gets every key, so Escape doesn't also close a dialog.
    e.preventDefault();
    e.stopPropagation();
    const page = Math.max(1, Math.floor((list?.clientHeight ?? ROW) / ROW) - 1);
    if (e.key === "Escape" || e.key === "Tab") hide();
    else if (e.key === "ArrowDown") move(active + 1);
    else if (e.key === "ArrowUp") move(active - 1);
    else if (e.key === "PageDown") move(active + page);
    else if (e.key === "PageUp") move(active - page);
    else if (e.key === "Home") move(0);
    else if (e.key === "End") move(options.length - 1);
    else if ((e.key === "Enter" || e.key === " ") && active >= 0) pick(active);
    else if (e.key.length === 1) {
      typed = e.timeStamp - typedAt > 700 ? e.key.toLowerCase() : typed + e.key.toLowerCase();
      typedAt = e.timeStamp;
      const at = options.findIndex(([, l]) => l.toLowerCase().startsWith(typed));
      if (at >= 0) move(at);
    }
  }

  function onPointerDown(e: PointerEvent) {
    const t = e.target as Node;
    if (!button.contains(t) && !list?.contains(t)) hide();
  }

  // The list is placed against the window, so a scroll elsewhere would leave it behind.
  function onScroll(e: Event) {
    if (!list?.contains(e.target as Node)) hide();
  }
</script>

<svelte:window
  onpointerdowncapture={open ? onPointerDown : undefined}
  onscrollcapture={open ? onScroll : undefined}
  onresize={open ? hide : undefined}
  onblur={open ? hide : undefined}
/>

<button
  bind:this={button}
  type="button"
  {id}
  class="select {cls}"
  class:open
  {disabled}
  role="combobox"
  aria-label={label}
  aria-haspopup="listbox"
  aria-expanded={open}
  aria-controls={uid}
  aria-activedescendant={open && active >= 0 ? `${uid}-${active}` : undefined}
  onclick={() => (open ? hide() : show())}
  onkeydown={onKeyDown}
>
  <span class="value">{shown}</span>
  <ChevronDown size={14} strokeWidth={2} />
</button>

{#if open}
  <div class="list" id={uid} role="listbox" bind:this={list} style={pos}>
    {#each options as [v, l], i}
      <!-- svelte-ignore a11y_click_events_have_key_events (the button above takes the keys) -->
      <div
        id="{uid}-{i}"
        class="opt"
        class:on={active === i}
        role="option"
        tabindex="-1"
        aria-selected={i === current}
        onpointerenter={() => (active = i)}
        onpointerdown={(e) => e.preventDefault()}
        onclick={() => pick(i)}
      >
        <span class="check">{#if i === current}<Check size={14} strokeWidth={2.2} />{/if}</span>
        <span class="label">{l}</span>
      </div>
    {/each}
  </div>
{/if}

<style>
  .select {
    display: inline-flex;
    align-items: center;
    justify-content: space-between;
    gap: 8px;
    height: 28px;
    max-width: 280px;
    padding: 0 8px 0 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: rgb(255 255 255 / 0.06);
    color: var(--text);
    font-size: 13px;
    text-align: left;
  }
  .select:hover:not(:disabled),
  .select.open {
    background: rgb(255 255 255 / 0.1);
  }
  .select:disabled {
    opacity: 0.5;
  }
  .select :global(svg) {
    flex-shrink: 0;
    color: var(--muted);
  }
  .value {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  /* The same panel as the title strip's menus. */
  .list {
    position: fixed;
    z-index: 200;
    max-width: min(420px, calc(100vw - 16px));
    overflow-y: auto;
    padding: 6px;
    border: 1px solid var(--border);
    border-radius: 12px;
    background: #1e1e21;
    box-shadow: 0 18px 50px rgb(0 0 0 / 0.55);
    box-sizing: border-box;
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 6px;
    height: 30px;
    padding: 0 12px 0 6px;
    border-radius: 7px;
    font-size: 13px;
    white-space: nowrap;
    cursor: default;
  }
  .opt.on {
    background: var(--hover);
  }
  .check {
    display: inline-grid;
    place-items: center;
    width: 16px;
    flex-shrink: 0;
    color: var(--text);
  }
  .label {
    overflow: hidden;
    text-overflow: ellipsis;
  }
</style>
