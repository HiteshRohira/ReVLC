<script module lang="ts">
  // Kept while the player has the window, so coming back lands on the same album.
  const at = $state({ mode: "albums" as "albums" | "songs", album: null as string | null });
</script>

<script lang="ts">
  import { ChevronLeft, Disc3, FolderPlus, Music2, Play } from "@lucide/svelte";
  import Page from "./Page.svelte";
  import { player, formatTime, thumbUrl, type Song } from "./player.svelte";

  let { onopenfolder }: { onopenfolder: () => void } = $props();

  interface Album {
    key: string;
    title: string;
    artist: string;
    year: number | null;
    art: string | null;
    songs: Song[];
    duration: number;
  }

  const base = (path: string) => path.split(/[\\/]/).filter(Boolean).pop() ?? path;
  const byName = (a: string, b: string) => a.localeCompare(b, undefined, { numeric: true, sensitivity: "base" });

  // Songs grouped into albums by their tags. Untagged songs make an album
  // of their folder, which is how such music is usually kept.
  const albums = $derived.by(() => {
    const map = new Map<string, Song[]>();
    for (const song of player.library.music) {
      // Without an album artist, two albums can share a title ("Greatest
      // Hits"); the folder tells them apart and keeps a compilation whole.
      const where = `${song.root}\u0000${song.folder}`;
      const key = !song.album
        ? `f\u0000${where}`
        : song.albumArtist
          ? `a\u0000${song.albumArtist.toLowerCase()}\u0000${song.album.toLowerCase()}`
          : `t\u0000${song.album.toLowerCase()}\u0000${where}`;
      if (!map.has(key)) map.set(key, []);
      map.get(key)!.push(song);
    }
    const list = [...map.entries()].map(([key, songs]): Album => {
      const first = songs[0];
      const sorted = [...songs].sort((a, b) => (a.disc ?? 1) - (b.disc ?? 1) || (a.track ?? 9999) - (b.track ?? 9999) || byName(a.name, b.name));
      // One artist names the album; several make it a compilation.
      const artists = new Set(songs.map((s) => s.artist).filter(Boolean));
      const artist = first.albumArtist ?? (artists.size === 1 ? [...artists][0]! : artists.size > 1 ? "Various artists" : "Unknown artist");
      return {
        key,
        title: first.album ?? (base(first.folder) || base(first.root)),
        artist,
        year: songs.find((s) => s.year)?.year ?? null,
        art: songs.find((s) => s.art)?.art ?? null,
        songs: sorted,
        duration: songs.reduce((total, s) => total + s.duration, 0),
      };
    });
    return list.sort((a, b) => byName(a.artist, b.artist) || byName(a.title, b.title));
  });

  const songs = $derived([...player.library.music].sort((a, b) => byName(a.name, b.name)));
  const open = $derived(albums.find((a) => a.key === at.album) ?? null);

  /** Plays a song and the ones after it in its list. */
  const playFrom = (list: Song[], index: number) => player.open(list.slice(index).map((s) => s.uri));

  const count = (n: number) => `${n} song${n === 1 ? "" : "s"}`;
  const length = (ms: number) => (ms >= 3_600_000 ? `${Math.floor(ms / 3_600_000)} h ${Math.round((ms % 3_600_000) / 60_000)} min` : `${Math.max(1, Math.round(ms / 60_000))} min`);
</script>

{#snippet cover(art: string | null, size: number)}
  {#if art}<img src={thumbUrl(art, player.thumbVersion)} alt="" />{:else}<Disc3 {size} strokeWidth={1.4} />{/if}
{/snippet}

<Page title="Music">
  {#snippet actions()}
    <button class="btn" onclick={onopenfolder}><FolderPlus size={16} />Add folder</button>
  {/snippet}

  {#if player.library.music.length === 0}
    <div class="blank">
      <Music2 size={44} strokeWidth={1.4} />
      <h2>{player.library.folders.length ? "No music in your library yet" : "Build your music library"}</h2>
      <p>Add a folder of music. ReVLC finds every song inside it and sorts them into albums by their tags.</p>
      <div class="btns"><button class="btn pri" onclick={onopenfolder}><FolderPlus size={16} />Add folder</button></div>
    </div>
  {:else if open}
    <div class="scroll">
      <div class="crumbs"><button onclick={() => (at.album = null)}><ChevronLeft size={15} />Albums</button></div>
      <div class="head">
        <div class="art big">{@render cover(open.art, 56)}</div>
        <div>
          <h2>{open.title}</h2>
          <p>{open.artist}{#if open.year}&nbsp;· {open.year}{/if}</p>
          <p class="sub">{count(open.songs.length)}{#if open.duration}&nbsp;· {length(open.duration)}{/if}</p>
          <button class="btn pri" onclick={() => playFrom(open.songs, 0)}><Play size={15} fill="currentColor" />Play</button>
        </div>
      </div>
      <ol class="rows" aria-label="Songs on {open.title}">
        {#each open.songs as song, i (song.uri)}
          <li class="row">
            <button class="main" data-tip={song.uri} onclick={() => playFrom(open.songs, i)}>
              <span class="num tnum">{song.track ?? i + 1}</span>
              <span class="t">{song.name}</span>
              <span class="d">{song.artist && song.artist !== open.artist ? song.artist : ""}</span>
              {#if song.duration}<span class="end tnum">{formatTime(song.duration)}</span>{/if}
            </button>
          </li>
        {/each}
      </ol>
    </div>
  {:else}
    <div class="scroll">
      <div class="chips" role="radiogroup" aria-label="Show">
        <button class="chip" class:on={at.mode === "albums"} role="radio" aria-checked={at.mode === "albums"} onclick={() => (at.mode = "albums")}>Albums</button>
        <button class="chip" class:on={at.mode === "songs"} role="radio" aria-checked={at.mode === "songs"} onclick={() => (at.mode = "songs")}>Songs</button>
      </div>
      {#if at.mode === "albums"}
        <div class="grid">
          {#each albums as album (album.key)}
            <div class="tile">
              <button class="art" aria-label="{album.title} by {album.artist}" onclick={() => (at.album = album.key)}>{@render cover(album.art, 40)}</button>
              <div class="n" data-tip={album.title}>{album.title}</div>
              <div class="m">{album.artist}</div>
            </div>
          {/each}
        </div>
      {:else}
        <ol class="rows" aria-label="Songs">
          {#each songs as song, i (song.uri)}
            <li class="row">
              <button class="main" data-tip={song.uri} onclick={() => playFrom(songs, i)}>
                <Music2 size={16} />
                <span class="t">{song.name}</span>
                <span class="d">{[song.artist, song.album].filter(Boolean).join(" · ")}</span>
                {#if song.duration}<span class="end tnum">{formatTime(song.duration)}</span>{/if}
              </button>
            </li>
          {/each}
        </ol>
      {/if}
    </div>
  {/if}
</Page>

<style>
  .grid {
    display: grid;
    grid-template-columns: repeat(auto-fill, minmax(150px, 1fr));
    gap: 22px 16px;
  }
  .tile {
    min-width: 0;
  }
  .art {
    display: grid;
    place-items: center;
    width: 100%;
    aspect-ratio: 1;
    border-radius: 10px;
    background: #1f1f23;
    color: var(--faint);
    overflow: hidden;
  }
  .art :global(img) {
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  button.art:hover {
    filter: brightness(1.12);
  }
  .art.big {
    flex: none;
    width: 168px;
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.4);
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
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .head {
    display: flex;
    align-items: flex-end;
    gap: 20px;
    margin-bottom: 20px;
  }
  .head h2 {
    margin: 0 0 4px;
    font-size: 24px;
  }
  .head p {
    margin: 0;
    color: var(--muted);
  }
  .head .sub {
    margin-top: 2px;
    font-size: 12.5px;
    color: var(--faint);
  }
  .head .btn {
    margin-top: 14px;
  }
</style>
