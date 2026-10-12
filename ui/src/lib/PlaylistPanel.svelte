<script lang="ts">
  import Icon from "./Icon.svelte";
  import { player, type Repeat } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let { onadd }: { onadd: () => void } = $props();

  let dragFrom = $state<number | null>(null);
  let dropAt = $state<number | null>(null);

  const NEXT_REPEAT: Record<Repeat, Repeat> = { off: "all", all: "one", one: "off" };
  const REPEAT_LABEL: Record<Repeat, string> = { off: "Repeat: off", all: "Repeat: all", one: "Repeat: one" };

  function onDrop(index: number) {
    if (dragFrom !== null && dragFrom !== index) player.move(dragFrom, index);
    dragFrom = dropAt = null;
  }
</script>

<aside class="panel" aria-label="Playlist">
  <header>
    <h2>Playlist <span class="count">{player.playlist.items.length}</span></h2>
    <div class="actions">
      <button class="icon-btn small" title="Add files (⌘O)" aria-label="Add files" onclick={onadd}>
        <Icon name="plus" size={18} />
      </button>
      <button
        class="icon-btn small"
        class:active={player.playlist.shuffle}
        title={player.playlist.shuffle ? "Shuffle: on" : "Shuffle: off"}
        aria-pressed={player.playlist.shuffle}
        aria-label="Shuffle"
        onclick={() => player.setShuffle(!player.playlist.shuffle)}
      >
        <Icon name="shuffle" size={18} />
      </button>
      <button
        class="icon-btn small repeat"
        class:active={player.playlist.repeat !== "off"}
        title={REPEAT_LABEL[player.playlist.repeat]}
        aria-label={REPEAT_LABEL[player.playlist.repeat]}
        onclick={() => player.setRepeat(NEXT_REPEAT[player.playlist.repeat])}
      >
        <Icon name="repeat" size={18} />
        {#if player.playlist.repeat === "one"}<span class="badge">1</span>{/if}
      </button>
      <button
        class="icon-btn small"
        title="Clear playlist"
        aria-label="Clear playlist"
        disabled={player.playlist.items.length === 0}
        onclick={player.clear}
      >
        <Icon name="trash" size={18} />
      </button>
      <button class="icon-btn small" title="Close" aria-label="Close playlist" onclick={() => (ui.playlistOpen = false)}>
        <Icon name="close" size={18} />
      </button>
    </div>
  </header>

  <ol>
    {#each player.playlist.items as item, i (i + item.uri)}
      <li
        class:current={i === player.playlist.current}
        class:drop-target={dropAt === i && dragFrom !== i}
        draggable="true"
        ondragstart={() => (dragFrom = i)}
        ondragover={(e) => {
          e.preventDefault();
          dropAt = i;
        }}
        ondrop={() => onDrop(i)}
        ondragend={() => (dragFrom = dropAt = null)}
      >
        <button class="entry" title={item.uri} ondblclick={() => player.playIndex(i)} onkeydown={(e) => e.key === "Enter" && player.playIndex(i)}>
          <span class="index">
            {#if i === player.playlist.current && player.playing}
              <Icon name="play" size={12} />
            {:else}
              {i + 1}
            {/if}
          </span>
          <span class="name">{item.title}</span>
        </button>
        <button class="remove" aria-label="Remove {item.title}" title="Remove" onclick={() => player.remove(i)}>
          <Icon name="close" size={14} />
        </button>
      </li>
    {:else}
      <p class="empty">Drop files here or press <kbd>⌘O</kbd> to add media.</p>
    {/each}
  </ol>
</aside>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    width: 320px;
    height: 100%;
    background: var(--panel);
    border-left: 1px solid var(--border);
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 44px 10px 8px 16px;
    border-bottom: 1px solid var(--border);
  }
  h2 {
    margin: 0;
    font-size: 14px;
    font-weight: 600;
  }
  .count {
    margin-left: 4px;
    color: var(--muted);
    font-weight: 400;
  }
  .actions {
    display: flex;
    gap: 0;
  }
  .repeat {
    position: relative;
  }
  .badge {
    position: absolute;
    right: 3px;
    bottom: 3px;
    font-size: 9px;
    font-weight: 700;
  }
  ol {
    flex: 1;
    margin: 0;
    padding: 6px;
    list-style: none;
    overflow-y: auto;
  }
  li {
    display: flex;
    align-items: center;
    border-radius: 8px;
    border-top: 2px solid transparent;
  }
  li:hover {
    background: var(--hover);
  }
  li.current {
    background: var(--accent-soft);
  }
  li.current .name {
    color: #fff;
    font-weight: 500;
  }
  li.drop-target {
    border-top-color: var(--accent);
  }
  .entry {
    display: flex;
    align-items: center;
    gap: 10px;
    flex: 1;
    min-width: 0;
    padding: 8px 8px;
    text-align: left;
    color: var(--text);
  }
  .index {
    width: 22px;
    flex-shrink: 0;
    text-align: right;
    font-size: 11px;
    color: var(--muted);
    font-variant-numeric: tabular-nums;
    display: flex;
    justify-content: flex-end;
  }
  li.current .index {
    color: var(--accent);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
  }
  .remove {
    padding: 6px;
    margin-right: 4px;
    border-radius: 6px;
    color: var(--muted);
    opacity: 0;
  }
  li:hover .remove,
  .remove:focus-visible {
    opacity: 1;
  }
  .remove:hover {
    color: var(--text);
    background: var(--hover-strong);
  }
  .empty {
    padding: 24px 16px;
    color: var(--muted);
    text-align: center;
    font-size: 13px;
  }
</style>
