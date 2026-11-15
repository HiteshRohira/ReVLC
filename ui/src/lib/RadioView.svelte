<script module lang="ts">
  import type { Station } from "./player.svelte";

  // Kept while the player has the window, so coming back shows the same stations.
  const at = $state({
    /** Favourites, or the directory. Null until the favourites are known. */
    tab: null as "favorites" | "directory" | null,
    query: "",
    tag: "",
    /** What the directory answered, and for which search. */
    found: null as { key: string; stations: Station[] } | null,
  });
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { Radio, RefreshCw, Search, Star, X } from "@lucide/svelte";
  import Page from "./Page.svelte";
  import { player } from "./player.svelte";

  const GENRES = ["Pop", "Rock", "Jazz", "Classical", "News", "Talk", "Electronic", "Hip hop", "Country", "Oldies", "Chillout", "Dance"];

  let typed = $state(at.query);
  let loading = $state(false);
  let error = $state<string | null>(null);
  /** The newest request: an answer to an older one is dropped. */
  let request = 0;

  const favorite = $derived(new Set(player.stations.map((s) => s.id)));
  const key = $derived(`${at.query}\u0000${at.tag}`);
  const stations = $derived(at.tab === "favorites" ? player.stations : at.found?.key === key ? at.found.stations : []);

  onMount(async () => {
    await player.loadStations();
    // Straight to your own stations if you have some; the directory otherwise.
    at.tab ??= player.stations.length ? "favorites" : "directory";
  });

  // The directory is asked when it's on show and the search has changed.
  $effect(() => {
    if (at.tab === "directory" && at.found?.key !== key) search();
  });

  async function search() {
    const id = ++request;
    const asked = key;
    loading = true;
    error = null;
    try {
      const found = await invoke<Station[]>("radio_stations", { query: at.query, tag: at.tag });
      if (id === request) at.found = { key: asked, stations: found };
    } catch (e) {
      if (id === request) {
        error = String(e);
        // Marked as answered, so it's asked again only when the user says.
        at.found = { key: asked, stations: [] };
      }
    } finally {
      if (id === request) loading = false;
    }
  }

  function submit(e: SubmitEvent) {
    e.preventDefault();
    at.query = typed.trim();
    at.tab = "directory";
  }

  function clear() {
    typed = "";
    at.query = "";
  }

  function pick(tag: string) {
    at.tag = at.tab === "directory" && at.tag === tag ? "" : tag;
    at.tab = "directory";
  }

  const detail = (s: Station) =>
    [s.country, [s.codec, s.bitrate ? `${s.bitrate} kbit/s` : ""].filter(Boolean).join(" "), s.tags.join(", ")].filter(Boolean).join(" · ");
</script>

<Page title="Internet radio">
  {#snippet actions()}
    <form class="search" role="search" onsubmit={submit}>
      <Search size={15} />
      <input type="text" bind:value={typed} placeholder="Search stations" spellcheck="false" autocomplete="off" aria-label="Search stations"
        onkeydown={(e) => e.key === "Escape" && typed && (clear(), e.stopPropagation())} />
      {#if typed}<button type="button" class="clear" aria-label="Clear search" onclick={clear}><X size={13} /></button>{/if}
    </form>
  {/snippet}

  <div class="scroll">
    <div class="chips" aria-label="Stations to show">
      <button class="chip" class:on={at.tab === "favorites"} aria-pressed={at.tab === "favorites"} onclick={() => (at.tab = "favorites")}>Favourites</button>
      <button class="chip" class:on={at.tab === "directory" && !at.tag} aria-pressed={at.tab === "directory" && !at.tag} onclick={() => ((at.tag = ""), (at.tab = "directory"))}>
        {at.query ? "All genres" : "Popular"}
      </button>
      {#each GENRES as genre}
        {@const on = at.tab === "directory" && at.tag === genre}
        <button class="chip" class:on aria-pressed={on} onclick={() => pick(genre)}>{genre}</button>
      {/each}
    </div>

    {#if at.tab === "directory" && loading}
      <p class="note" role="status">Asking the radio directory…</p>
    {:else if at.tab === "directory" && error}
      <p class="note error" role="alert">{error}</p>
      <button class="btn small" onclick={search}><RefreshCw size={13} />Try again</button>
    {:else if stations.length}
      <ul class="rows" aria-label={at.tab === "favorites" ? "Favourite stations" : "Stations"}>
        {#each stations as station (station.id)}
          {@const starred = favorite.has(station.id)}
          <li class="row">
            <button class="main" data-tip={station.url} onclick={() => player.playStation(station)}>
              <Radio size={17} />
              <span class="t">{station.name}</span>
              <span class="d">{detail(station)}</span>
            </button>
            <button class="act" class:on={starred} aria-pressed={starred} aria-label="{starred ? 'Remove' : 'Add'} {station.name} {starred ? 'from' : 'to'} favourites"
              data-tip={starred ? "Remove from favourites" : "Add to favourites"} onclick={() => player.setFavoriteStation(station, !starred)}>
              <Star size={15} fill={starred ? "currentColor" : "none"} />
            </button>
          </li>
        {/each}
      </ul>
    {:else if at.tab === "favorites"}
      <p class="note">No favourites yet. Star a station to keep it here.</p>
    {:else if at.tab === "directory"}
      <p class="note">No stations match that search.</p>
    {/if}

    {#if at.tab === "directory"}
      <p class="credit">Stations come from radio-browser.info, a free community directory. Your searches are sent to it; nothing else is.</p>
    {/if}
  </div>
</Page>

<style>
  .search {
    position: relative;
    display: flex;
    align-items: center;
    width: 260px;
    color: var(--muted);
  }
  .search :global(svg) {
    position: absolute;
    left: 10px;
    pointer-events: none;
  }
  .search input {
    box-sizing: border-box;
    width: 100%;
    height: 34px;
    padding: 0 30px 0 32px;
    border-radius: 9px;
    border: 1px solid var(--border);
    background: rgb(0 0 0 / 0.3);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    outline: none;
    -webkit-user-select: text;
    user-select: text;
  }
  .search input:focus {
    border-color: var(--acc);
  }
  .clear {
    position: absolute;
    right: 6px;
    width: 22px;
    height: 22px;
    border-radius: 50%;
    color: var(--muted);
  }
  .clear :global(svg) {
    position: static;
  }
  .clear:hover {
    background: var(--hover-strong);
    color: var(--text);
  }
  .credit {
    margin: 18px 10px 0;
    font-size: 12px;
    color: var(--faint);
  }
</style>
