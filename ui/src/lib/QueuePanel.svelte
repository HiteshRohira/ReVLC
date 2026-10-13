<script lang="ts">
  import { Plus, Repeat, Repeat1, Shuffle, Trash2, X } from "@lucide/svelte";
  import { player, cleanTitle, type Repeat as RepeatMode } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let { onadd }: { onadd: () => void } = $props();

  let dragFrom = $state<number | null>(null);
  let dropAt = $state<number | null>(null);

  const NEXT: Record<RepeatMode, RepeatMode> = { off: "all", all: "one", one: "off" };
  const LABEL: Record<RepeatMode, string> = { off: "Repeat off", all: "Repeat all", one: "Repeat one" };

  function onDrop(index: number) {
    if (dragFrom !== null && dragFrom !== index) player.move(dragFrom, index);
    dragFrom = dropAt = null;
  }
</script>

<aside class="queue" aria-label="Up next">
  <header>
    <h2>Up next <span>{player.playlist.items.length}</span></h2>
    <button class="icon" title="Close (⌘L)" aria-label="Close" onclick={() => (ui.playlistOpen = false)}><X size={18} /></button>
  </header>
  <div class="tools">
    <button class="tool" onclick={onadd}><Plus size={15} /> Add</button>
    <span class="sp"></span>
    <button class="icon sm" class:on={player.playlist.shuffle} title={player.playlist.shuffle ? "Shuffle on" : "Shuffle off"}
      aria-pressed={player.playlist.shuffle} onclick={() => player.setShuffle(!player.playlist.shuffle)}><Shuffle size={16} /></button>
    <button class="icon sm" class:on={player.playlist.repeat !== "off"} title={LABEL[player.playlist.repeat]} aria-label={LABEL[player.playlist.repeat]}
      onclick={() => player.setRepeat(NEXT[player.playlist.repeat])}>
      {#if player.playlist.repeat === "one"}<Repeat1 size={16} />{:else}<Repeat size={16} />{/if}
    </button>
    <button class="icon sm" title="Clear" aria-label="Clear queue" disabled={!player.playlist.items.length} onclick={player.clear}><Trash2 size={16} /></button>
  </div>
  <ol>
    {#each player.playlist.items as item, i (i + item.uri)}
      <li class:current={i === player.playlist.current} class:target={dropAt === i && dragFrom !== i} draggable="true"
        ondragstart={() => (dragFrom = i)} ondragover={(e) => { e.preventDefault(); dropAt = i; }} ondrop={() => onDrop(i)}
        ondragend={() => (dragFrom = dropAt = null)}>
        <button class="entry" title={item.uri} ondblclick={() => player.playIndex(i)} onkeydown={(e) => e.key === "Enter" && player.playIndex(i)}>
          <span class="n tnum">{i === player.playlist.current ? "▶" : i + 1}</span>
          <span class="t">{cleanTitle(item.title)}</span>
        </button>
        <button class="rm" aria-label="Remove {item.title}" title="Remove" onclick={() => player.remove(i)}><X size={14} /></button>
      </li>
    {:else}
      <p class="empty">Drop files here or click Add.</p>
    {/each}
  </ol>
</aside>

<style>
  .queue {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 340px;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-left: 1px solid var(--border);
    box-shadow: -12px 0 40px rgb(0 0 0 / 0.35);
    z-index: 20;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 44px 12px 6px 18px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 700;
  }
  h2 span {
    margin-left: 4px;
    font-weight: 400;
    color: var(--muted);
  }
  .tools {
    display: flex;
    align-items: center;
    gap: 2px;
    padding: 4px 12px 10px 14px;
    border-bottom: 1px solid var(--border);
  }
  .sp {
    flex: 1;
  }
  .tool {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px;
    border-radius: 8px;
    background: var(--hover);
    font-size: 12.5px;
    font-weight: 600;
  }
  .icon {
    width: 32px;
    height: 32px;
    border-radius: 8px;
    color: var(--text);
  }
  .icon:hover:not(:disabled),
  .tool:hover {
    background: var(--hover-strong);
  }
  .icon.on {
    color: var(--acc);
  }
  ol {
    flex: 1;
    margin: 0;
    padding: 8px;
    list-style: none;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    border-radius: 9px;
    border-top: 2px solid transparent;
  }
  li:hover {
    background: var(--hover);
  }
  li.current {
    background: var(--acc-soft);
  }
  li.target {
    border-top-color: var(--acc);
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 12px;
    flex: 1;
    min-width: 0;
    padding: 10px;
    text-align: left;
  }
  .n {
    width: 20px;
    text-align: right;
    font-size: 11.5px;
    color: var(--muted);
  }
  li.current .n {
    color: var(--acc);
  }
  .t {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13.5px;
  }
  li.current .t {
    font-weight: 600;
  }
  .rm {
    padding: 6px;
    margin-right: 4px;
    border-radius: 6px;
    color: var(--muted);
    opacity: 0;
  }
  li:hover .rm,
  .rm:focus-visible {
    opacity: 1;
  }
  .empty {
    padding: 28px 16px;
    text-align: center;
    color: var(--muted);
    font-size: 13px;
  }
</style>
