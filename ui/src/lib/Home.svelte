<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { Clapperboard, Disc3, Film, FolderOpen, FolderPlus, HardDrive, House, Link, ListMusic, Music2, Play, Radio, Settings, X } from "@lucide/svelte";
  import { player, resumable, formatTime, thumbUrl, cleanTitle, type HistoryEntry, type LibraryItem } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let { onopen, onopenfolder }: { onopen: () => void; onopenfolder: () => void } = $props();

  let view = $state<"home" | "videos">("home");

  const continueWatching = $derived(player.history.filter(resumable).slice(0, 4));
  const recent = $derived(player.history.filter((e) => !resumable(e)).slice(0, 8));
  const recentlyAdded = $derived([...player.library.items].sort((a, b) => b.modified - a.modified).slice(0, 8));
  const isEmpty = $derived(player.history.length === 0 && player.library.items.length === 0);

  // Library grouped by folder ("Season 01", …), in scan order.
  const groups = $derived.by(() => {
    const map = new Map<string, { root: string; folder: string; items: LibraryItem[] }>();
    for (const item of player.library.items) {
      const key = `${item.root}\u0000${item.folder}`;
      if (!map.has(key)) map.set(key, { root: item.root, folder: item.folder, items: [] });
      map.get(key)!.items.push(item);
    }
    return [...map.values()];
  });

  // Planned sections from the parity checklist; shown so the roadmap is visible.
  const SOON = [
    { label: "Music", icon: Music2 },
    { label: "Playlists", icon: ListMusic },
  ];
  const SOON_BROWSE = [
    { label: "Network shares", icon: HardDrive },
    { label: "Internet radio", icon: Radio },
    { label: "Discs", icon: Disc3 },
  ];

  const base = (path: string) => path.split("/").filter(Boolean).pop() ?? path;
  const left = (e: HistoryEntry) => `${Math.max(1, Math.round((e.length - e.position) / 60000))} min left`;

  /** Plays a library file and the rest of its folder after it. */
  function playFromFolder(item: LibraryItem) {
    const group = groups.find((g) => g.root === item.root && g.folder === item.folder);
    const items = group?.items ?? [item];
    player.open(items.slice(items.indexOf(item)).map((i) => i.uri));
  }
</script>

{#snippet thumb(src: string | null, length: number, progress: number | null, onclick: () => void, title: string)}
  <button class="th" style={src ? `background-image:url("${src}")` : ""} {title} {onclick}>
    {#if !src}<Clapperboard size={28} strokeWidth={1.5} />{/if}
    <span class="hover"><Play size={26} fill="currentColor" /></span>
    {#if length > 0}<span class="dur tnum">{formatTime(length)}</span>{/if}
    {#if progress !== null}<span class="pg"><b style="width: {progress * 100}%"></b></span>{/if}
  </button>
{/snippet}

{#snippet historyTile(e: HistoryEntry, withProgress: boolean)}
  <div class="tile">
    {@render thumb(thumbUrl(e.thumb, player.thumbVersion), e.length, withProgress && e.length > 0 ? e.position / e.length : null, () => player.open([e.uri]), e.uri)}
    <button class="forget" title="Remove from history" aria-label="Remove {cleanTitle(e.title)} from history" onclick={() => player.forget(e.uri)}><X size={14} /></button>
    <div class="n" title={cleanTitle(e.title)}>{cleanTitle(e.title)}</div>
    <div class="m">{withProgress ? left(e) : e.finished ? "Watched" : e.uri.includes("://") ? "Stream" : "Recently opened"}</div>
  </div>
{/snippet}

{#snippet libraryTile(item: LibraryItem, sub: string)}
  <div class="tile">
    {@render thumb(thumbUrl(item.thumb, player.thumbVersion), 0, null, () => playFromFolder(item), item.uri)}
    <div class="n" title={item.title}>{item.title}</div>
    <div class="m">{sub}</div>
  </div>
{/snippet}

{#snippet dropZone(title: string, text: string)}
  <div class="drop">
    <Clapperboard size={44} strokeWidth={1.4} />
    <h2>{title}</h2>
    <p>{text}</p>
    <div class="row">
      <button class="btn pri" onclick={onopen}><FolderOpen size={16} />Open…</button>
      <button class="btn" onclick={onopenfolder}><FolderPlus size={16} />Add folder</button>
      <button class="btn" onclick={() => (ui.urlOpen = true)}><Link size={16} />Open URL</button>
    </div>
    <button class="hint" onclick={() => (ui.helpOpen = true)}>Keyboard shortcuts <kbd>?</kbd></button>
  </div>
{/snippet}

<div class="home">
  <aside data-tauri-drag-region>
    <nav>
      <button class="nv" class:on={view === "home"} onclick={() => (view = "home")}><House size={17} />Home</button>
      <button class="nv" class:on={view === "videos"} onclick={() => (view = "videos")}>
        <Film size={17} />Videos{#if player.library.items.length}<em>{player.library.items.length}</em>{/if}
      </button>
      {#each SOON as s}<div class="nv soon"><s.icon size={17} />{s.label}<span>Soon</span></div>{/each}
      <h6>Browse</h6>
      {#each SOON_BROWSE as s}<div class="nv soon"><s.icon size={17} />{s.label}<span>Soon</span></div>{/each}
      <h6>Streams</h6>
      <button class="nv" onclick={() => (ui.urlOpen = true)}><Link size={17} />Open URL…</button>
    </nav>
    <div class="appearance">
      <button class="nv" onclick={() => invoke("open_settings")}><Settings size={17} />Settings</button>
    </div>
  </aside>

  <main>
    <div class="top" data-tauri-drag-region>
      <h1 data-tauri-drag-region>{view === "home" ? "Home" : "Videos"}</h1>
      <span class="sp" data-tauri-drag-region></span>
      <button class="btn" onclick={() => (ui.urlOpen = true)}><Link size={16} />Open URL</button>
      <button class="btn" onclick={onopenfolder}><FolderPlus size={16} />Add folder</button>
      <button class="btn pri" onclick={onopen}><FolderOpen size={16} />Open…</button>
    </div>

    {#if player.engineError}
      <div class="drop error"><h2>libVLC not found</h2><p>{player.engineError}</p></div>
    {:else if view === "home"}
      {#if isEmpty}
        {@render dropZone("Drop a video here", "or open a file, add a folder to your library, or play a network stream")}
      {:else}
        <div class="scroll">
          {#if continueWatching.length}
            <h2>Continue watching</h2>
            <div class="grid">{#each continueWatching as e (e.uri)}{@render historyTile(e, true)}{/each}</div>
          {/if}
          {#if recentlyAdded.length}
            <h2>Recently added</h2>
            <div class="grid">{#each recentlyAdded as item (item.uri)}{@render libraryTile(item, item.folder || base(item.root))}{/each}</div>
          {/if}
          {#if recent.length}
            <h2>Recent</h2>
            <div class="grid">{#each recent as e (e.uri)}{@render historyTile(e, false)}{/each}</div>
          {/if}
          <p class="foot">Drop files or folders anywhere to play them.</p>
        </div>
      {/if}
    {:else if player.library.folders.length === 0}
      {@render dropZone("Build your library", "Add a folder of videos. ReVLC finds every video inside it, including subfolders like seasons.")}
    {:else}
      <div class="scroll">
        {#each player.library.folders as root (root)}
          {@const rootGroups = groups.filter((g) => g.root === root)}
          <div class="root">
            <div>
              <h2>{base(root)}</h2>
              <p title={root}>{root}</p>
            </div>
            <button class="btn small" title="Remove from library (files are not deleted)" onclick={() => player.removeFolder(root)}><X size={14} />Remove</button>
          </div>
          {#each rootGroups as g (g.folder)}
            {#if g.folder}<h3>{g.folder}</h3>{/if}
            <div class="grid">{#each g.items as item (item.uri)}{@render libraryTile(item, "")}{/each}</div>
          {:else}
            <p class="empty">No videos found in this folder.</p>
          {/each}
        {/each}
      </div>
    {/if}
  </main>
</div>

<style>
  .home {
    position: absolute;
    inset: 0;
    display: grid;
    grid-template-columns: 220px 1fr;
    background: var(--bg);
  }
  aside {
    display: flex;
    flex-direction: column;
    justify-content: space-between;
    padding: 52px 12px 16px;
    background: var(--sidebar);
    border-right: 1px solid var(--border);
  }
  h6 {
    margin: 18px 10px 6px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--faint);
  }
  .nv {
    display: flex;
    justify-content: flex-start;
    align-items: center;
    gap: 10px;
    width: 100%;
    padding: 7px 10px;
    border-radius: 8px;
    font-size: 13.5px;
    color: var(--text);
    text-align: left;
  }
  button.nv:hover {
    background: var(--hover);
  }
  .nv :global(svg) {
    color: var(--muted);
  }
  .nv.on {
    background: var(--acc-soft);
  }
  .nv.on :global(svg) {
    color: var(--acc);
  }
  .nv em {
    margin-left: auto;
    font-style: normal;
    font-size: 12px;
    color: var(--muted);
  }
  .nv.soon,
  .nv.soon :global(svg) {
    color: var(--faint);
  }
  .nv span {
    margin-left: auto;
    margin-right: -2px;
    font-size: 10.5px;
    font-weight: 600;
    padding: 1px 6px;
    border-radius: 5px;
    background: var(--hover);
    color: var(--faint);
  }
  main {
    position: relative;
    display: flex;
    flex-direction: column;
    min-width: 0;
    min-height: 0;
  }
  .top {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 64px;
    padding: 0 24px 0 28px;
  }
  .top h1 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
  }
  .sp {
    flex: 1;
    align-self: stretch;
  }
  .btn.small {
    height: 28px;
    padding: 0 10px;
    font-size: 12px;
  }
  .drop {
    flex: 1;
    margin: 0 24px 24px;
    border: 1.5px dashed rgb(255 255 255 / 0.14);
    border-radius: 18px;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 8px;
    color: var(--faint);
    text-align: center;
  }
  .drop h2 {
    margin: 8px 0 0;
    font-size: 20px;
    color: var(--text);
  }
  .drop p {
    margin: 0;
    color: var(--muted);
    max-width: 440px;
  }
  .drop .row {
    display: flex;
    gap: 10px;
    margin-top: 14px;
  }
  .hint {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 22px;
    font-size: 12px;
    color: var(--faint);
  }
  .scroll {
    flex: 1;
    overflow-y: auto;
    padding: 4px 28px 24px;
  }
  .scroll h2 {
    margin: 6px 0 14px;
    font-size: 18px;
    font-weight: 700;
  }
  .scroll .grid {
    margin-bottom: 30px;
  }
  .root {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 16px;
    margin: 4px 0 12px;
  }
  .root h2 {
    margin: 0;
  }
  .root p {
    margin: 2px 0 0;
    font-size: 12px;
    color: var(--faint);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  h3 {
    margin: 0 0 12px;
    font-size: 14px;
    font-weight: 600;
    color: var(--muted);
  }
  .empty {
    color: var(--muted);
  }
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
    gap: 20px 16px;
  }
  .tile {
    position: relative;
    min-width: 0;
  }
  .th {
    position: relative;
    display: grid;
    place-items: center;
    width: 100%;
    aspect-ratio: 16 / 9;
    border-radius: 10px;
    background: #1f1f23 center / cover;
    color: var(--faint);
    overflow: hidden;
  }
  .hover {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    color: #fff;
    background: rgb(0 0 0 / 0.35);
    opacity: 0;
    transition: opacity 0.15s;
  }
  .th:hover .hover {
    opacity: 1;
  }
  .dur {
    position: absolute;
    right: 7px;
    bottom: 9px;
    padding: 1px 6px;
    border-radius: 5px;
    background: rgb(0 0 0 / 0.75);
    color: #fff;
    font-size: 11px;
    font-weight: 600;
  }
  .pg {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    height: 3px;
    background: rgb(255 255 255 / 0.25);
  }
  .pg b {
    display: block;
    height: 100%;
    background: var(--acc);
  }
  .forget {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 24px;
    height: 24px;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.65);
    color: #fff;
    opacity: 0;
  }
  .tile:hover .forget,
  .forget:focus-visible {
    opacity: 1;
  }
  .n {
    margin-top: 9px;
    font-size: 13.5px;
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .m {
    margin-top: 2px;
    font-size: 12px;
    color: var(--muted);
    min-height: 1em;
  }
  .foot {
    margin: 0;
    font-size: 12px;
    color: var(--faint);
  }
  .error p {
    font-size: 13px;
  }
</style>
