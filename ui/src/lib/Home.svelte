<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { Clapperboard, Disc3, Film, FolderOpen, FolderPlus, HardDrive, House, Link, ListMusic, Music2, Play, Radio, Settings, Webcam, X } from "@lucide/svelte";
  import { player, resumable, formatTime, thumbUrl, cleanTitle, type HistoryEntry, type LibraryItem } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let { onopen, onopenfolder }: { onopen: () => void; onopenfolder: () => void } = $props();

  let view = $state<"home" | "videos">("home");

  const continueWatching = $derived(player.history.filter(resumable).slice(0, 4));
  const recent = $derived(player.history.filter((e) => !resumable(e)).slice(0, 8));
  const recentlyAdded = $derived([...player.library.items].sort((a, b) => b.modified - a.modified).slice(0, 8));
  const isEmpty = $derived(player.history.length === 0 && player.library.items.length === 0);

  interface Group {
    key: string;
    root: string;
    /** Show name or folder ("" for files directly in the library folder). */
    title: string;
    detail: string;
    art: string | null;
    sections: { label: string | null; items: LibraryItem[] }[];
    /** Every item, in display order. */
    items: LibraryItem[];
  }

  // Library grouped into shows (by season) and folders, in scan order.
  const groups = $derived.by(() => {
    const map = new Map<string, { root: string; show: string | null; folder: string; items: LibraryItem[] }>();
    for (const item of player.library.items) {
      const key = item.show ? `${item.root}\u0000show\u0000${item.show.toLowerCase()}` : `${item.root}\u0000${item.folder}`;
      if (!map.has(key)) map.set(key, { root: item.root, show: item.show, folder: item.folder, items: [] });
      map.get(key)!.items.push(item);
    }
    return [...map.entries()].map(([key, g]): Group => {
      const art = g.items.find((i) => i.groupArt)?.groupArt ?? null;
      if (!g.show) {
        return { key, root: g.root, title: g.folder, detail: "", art, sections: [{ label: null, items: g.items }], items: g.items };
      }
      const seasons = new Map<number | null, LibraryItem[]>();
      const sorted = [...g.items].sort((a, b) => (a.season ?? 0) - (b.season ?? 0) || (a.episode ?? 0) - (b.episode ?? 0));
      for (const item of sorted) {
        if (!seasons.has(item.season)) seasons.set(item.season, []);
        seasons.get(item.season)!.push(item);
      }
      const n = seasons.size;
      return {
        key,
        root: g.root,
        title: g.show,
        detail: `${n > 1 ? `${n} seasons · ` : ""}${sorted.length} episode${sorted.length === 1 ? "" : "s"}`,
        art,
        sections: [...seasons.entries()].map(([season, items]) => ({ label: season === null ? "Episodes" : season === 0 ? "Specials" : `Season ${season}`, items })),
        items: sorted,
      };
    });
  });

  // Planned sections from the parity checklist; shown so the roadmap is visible.
  const SOON = [
    { label: "Music", icon: Music2 },
    { label: "Playlists", icon: ListMusic },
  ];
  const SOON_BROWSE = [
    { label: "Network shares", icon: HardDrive },
    { label: "Internet radio", icon: Radio },
  ];

  const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const left = (e: HistoryEntry) => `${Math.max(1, Math.round((e.length - e.position) / 60000))} min left`;

  /** Plays a library file and the rest of its show or folder after it. */
  function playFromFolder(item: LibraryItem) {
    const items = groups.find((g) => g.items.includes(item))?.items ?? [item];
    player.open(items.slice(items.indexOf(item)).map((i) => i.uri));
  }

  const episodeCode = (i: LibraryItem) => (i.season !== null && i.episode !== null ? `S${i.season}·E${i.episode}` : i.episode !== null ? `E${i.episode}` : "");
  /** Tile title: the episode's number and title, or the movie's name. */
  const tileName = (i: LibraryItem, inShow: boolean) =>
    i.show && inShow ? (i.episode !== null ? `${i.episode}. ${i.name || `Episode ${i.episode}`}` : i.name || i.title) : i.name || i.title;
  /** Tile subtitle outside a show's own section: where it's from. */
  const tileFrom = (i: LibraryItem) => (i.show ? `${i.show} · ${episodeCode(i)}` : [i.year, i.folder || base(i.root)].filter(Boolean).join(" · "));
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
    <button class="forget" data-tip="Remove from history" aria-label="Remove {cleanTitle(e.title)} from history" onclick={() => player.forget(e.uri)}><X size={14} /></button>
    <div class="n" data-tip={cleanTitle(e.title)}>{cleanTitle(e.title)}</div>
    <div class="m">{withProgress ? left(e) : e.finished ? "Watched" : e.uri.includes("://") ? "Stream" : "Recently opened"}</div>
  </div>
{/snippet}

{#snippet libraryTile(item: LibraryItem, name: string, sub: string)}
  <div class="tile">
    {@render thumb(thumbUrl(item.thumb ?? item.art, player.thumbVersion), item.duration, null, () => playFromFolder(item), item.uri)}
    <div class="n" data-tip={name}>{name}</div>
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
      <button class="btn" onclick={() => (ui.dialog = "url")}><Link size={16} />Open URL</button>
      <button class="btn" onclick={() => (ui.dialog = "disc")}><Disc3 size={16} />Open disc</button>
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
      <button class="nv" onclick={() => (ui.dialog = "disc")}><Disc3 size={17} />Discs…</button>
      {#each SOON_BROWSE as s}<div class="nv soon"><s.icon size={17} />{s.label}<span>Soon</span></div>{/each}
      <h6>Streams</h6>
      <button class="nv" onclick={() => (ui.dialog = "url")}><Link size={17} />Open URL…</button>
      <button class="nv" onclick={() => (ui.dialog = "capture")}><Webcam size={17} />Capture device…</button>
    </nav>
    <div class="appearance">
      <button class="nv" onclick={() => invoke("open_settings")}><Settings size={17} />Settings</button>
    </div>
  </aside>

  <main>
    <div class="top" data-tauri-drag-region>
      <h1 data-tauri-drag-region>{view === "home" ? "Home" : "Videos"}</h1>
      <span class="sp" data-tauri-drag-region></span>
      <button class="btn" onclick={() => (ui.dialog = "url")}><Link size={16} />Open URL</button>
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
            <div class="grid">{#each recentlyAdded as item (item.uri)}{@render libraryTile(item, item.show ? item.show : tileName(item, false), item.show ? `${episodeCode(item)}${item.name ? ` · ${item.name}` : ""}` : tileFrom(item))}{/each}</div>
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
              <p data-tip={root}>{root}</p>
            </div>
            <button class="btn small" data-tip="Remove from library (files are not deleted)" aria-label="Remove {root} from library" onclick={() => player.removeFolder(root)}><X size={14} />Remove</button>
          </div>
          {#each rootGroups as g (g.key)}
            {#if g.art}
              <div class="ghead">
                <img class="poster" src={thumbUrl(g.art, player.thumbVersion)} alt="" />
                <div><h3>{g.title || base(g.root)}</h3>{#if g.detail}<p>{g.detail}</p>{/if}</div>
              </div>
            {:else if g.title}
              <h3>{g.title}{#if g.detail}<span class="detail">&nbsp;· {g.detail}</span>{/if}</h3>
            {/if}
            {#each g.sections as sec (sec.label)}
              {#if sec.label && g.sections.length > 1}<h4>{sec.label}</h4>{/if}
              <div class="grid">
                {#each sec.items as item (item.uri)}{@render libraryTile(item, tileName(item, true), item.show ? "" : item.year ? String(item.year) : "")}{/each}
              </div>
            {/each}
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
  /* Below the title strip, with no traffic lights to clear. */
  :global([data-platform="windows"]) .home {
    top: 36px;
  }
  :global([data-platform="windows"]) aside {
    padding-top: 16px;
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
    box-sizing: border-box;
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
  h3 .detail {
    font-weight: 400;
    color: var(--faint);
  }
  h4 {
    margin: 2px 0 10px;
    font-size: 12.5px;
    font-weight: 600;
    color: var(--faint);
  }
  .ghead {
    display: flex;
    align-items: flex-end;
    gap: 14px;
    margin-bottom: 14px;
  }
  .ghead h3 {
    margin: 0;
    font-size: 16px;
    color: var(--text);
  }
  .ghead p {
    margin: 3px 0 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .poster {
    width: 72px;
    aspect-ratio: 2 / 3;
    object-fit: cover;
    border-radius: 7px;
    background: #1f1f23;
    box-shadow: 0 4px 14px rgb(0 0 0 / 0.4);
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
