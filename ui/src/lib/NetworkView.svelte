<script module lang="ts">
  import type { Listing } from "./player.svelte";

  // Kept while the player has the window, so coming back lands in the same folder.
  const at = $state({
    /** The way here: each folder opened, with the name it was opened under. */
    trail: [] as { name: string; location: string }[],
    listing: null as Listing | null,
  });
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { ChevronRight, Film, Folder, FolderPlus, HardDrive, LogIn, Music2, Play, Plus, RefreshCw, Server, Wifi, X } from "@lucide/svelte";
  import Dialog from "./Dialog.svelte";
  import Page from "./Page.svelte";
  import { isMac, isWindows } from "./platform";
  import { player, type BrowseEntry, type Places } from "./player.svelte";
  import { ui } from "./ui.svelte";

  const AUDIO = /\.(mp3|m4a|m4b|aac|flac|wav|ogg|oga|opus|wma|aiff?|ape|mka|wv)$/i;

  let places = $state<Places>({ mounted: [], servers: [], found: [] });
  let loading = $state(false);
  let error = $state<string | null>(null);
  /** The newest request: an answer to an older one is dropped. */
  let request = 0;

  // The "Connect to server" box.
  let address = $state("");
  let user = $state("");
  let password = $state("");
  let remember = $state(false);
  let input = $state<HTMLInputElement>();

  const here = $derived(at.trail[at.trail.length - 1] ?? null);
  const files = $derived(at.listing?.entries.filter((e) => !e.dir) ?? []);
  const scheme = $derived(address.trim().match(/^([a-z0-9]+):\/\//i)?.[1].toLowerCase() ?? "smb");
  // The system signs in to these itself, with its own window and keychain.
  const systemSignIn = $derived((isMac && (scheme === "smb" || scheme === "afp")) || (isWindows && scheme === "smb"));
  const signsIn = $derived(!systemSignIn && scheme !== "upnp" && scheme !== "nfs");

  async function loadPlaces() {
    try {
      places = await invoke<Places>("network_places");
    } catch {
      // The engine isn't there; Home says so.
    }
  }

  onMount(() => {
    loadPlaces();
    // Servers turn up over a few seconds, and shares get mounted and ejected.
    const timer = setInterval(() => !here && loadPlaces(), 3000);
    return () => {
      clearInterval(timer);
      invoke("network_leave").catch(() => {});
    };
  });

  $effect(() => {
    if (ui.dialog === "server") input?.focus();
  });

  /** Opens a folder. `trail` is the way to it, itself included. */
  async function browse(trail: { name: string; location: string }[], sign?: { user: string; password: string }) {
    const target = trail[trail.length - 1];
    const id = ++request;
    loading = true;
    error = null;
    at.trail = trail;
    at.listing = null;
    try {
      const listing = await invoke<Listing>("network_browse", { location: target.location, user: sign?.user ?? null, password: sign?.password ?? null });
      if (id !== request) return;
      at.listing = listing;
    } catch (e) {
      if (id !== request) return;
      // Cancelling the system's sign-in window isn't a failure to report.
      if (String(e) === "Cancelled") home();
      else error = String(e);
    } finally {
      if (id === request) loading = false;
    }
  }

  function home() {
    request++;
    loading = false;
    error = null;
    at.trail = [];
    at.listing = null;
    loadPlaces();
  }

  function open(entry: BrowseEntry) {
    if (entry.dir) return browse([...at.trail, { name: entry.name, location: entry.uri }]);
    // This file, then the rest of the folder after it.
    const from = files.findIndex((f) => f.uri === entry.uri);
    return player.open(files.slice(from).map((f) => f.uri));
  }

  function connect(url = "", name = "") {
    address = url;
    user = name;
    password = "";
    remember = false;
    ui.dialog = "server";
  }

  async function submit() {
    const url = address.trim();
    ui.dialog = null;
    if (remember) {
      try {
        places.servers = await invoke<Places["servers"]>("network_save",{ url, name: "", user: signsIn && user.trim() ? user.trim() : null });
      } catch (e) {
        player.notify("error", String(e));
      }
    }
    const name = url.replace(/^[a-z0-9]+:\/\/([^/@]*@)?/i, "").replace(/\/+$/, "") || url;
    await browse([{ name, location: url }], signsIn ? { user, password } : undefined);
    password = "";
  }

  async function forget(url: string) {
    try {
      places.servers = await invoke<Places["servers"]>("network_forget",{ url });
    } catch (e) {
      player.notify("error", String(e));
    }
  }

  async function addToLibrary() {
    if (!at.listing) return;
    await player.addFolder(at.listing.location);
    player.notify("info", "Added to your library");
  }
</script>

<Page title="Network shares">
  {#snippet actions()}
    <button class="btn pri" onclick={() => connect()}><Plus size={16} />Connect to server…</button>
  {/snippet}

  {#if here}
    <div class="scroll">
      <div class="crumbs" aria-label="Path">
        <button onclick={home}>Network shares</button>
        {#each at.trail as step, i (i)}
          <ChevronRight size={13} />
          {#if i === at.trail.length - 1}<b>{step.name}</b>{:else}<button onclick={() => browse(at.trail.slice(0, i + 1))}>{step.name}</button>{/if}
        {/each}
      </div>
      {#if loading}
        <p class="note" role="status">Opening {here.name}…</p>
      {:else if error}
        <p class="note error" role="alert">{error}</p>
        <div class="tools">
          <button class="btn small" onclick={() => browse(at.trail)}><RefreshCw size={13} />Try again</button>
          {#if here.location.includes("://")}<button class="btn small" onclick={() => connect(here.location)}><LogIn size={13} />Sign in…</button>{/if}
        </div>
      {:else if at.listing}
        <div class="tools">
          <button class="btn small" disabled={!files.length} onclick={() => player.open(files.map((f) => f.uri))}><Play size={13} fill="currentColor" />Play all</button>
          {#if at.listing.local}<button class="btn small" onclick={addToLibrary}><FolderPlus size={13} />Add to library</button>{/if}
        </div>
        <ul class="rows" aria-label="Contents of {here.name}">
          {#each at.listing.entries as entry (entry.uri)}
            <li class="row">
              <button class="main" data-tip={entry.dir ? null : entry.name} onclick={() => open(entry)}>
                {#if entry.dir}<Folder size={17} />{:else if AUDIO.test(entry.name)}<Music2 size={17} />{:else}<Film size={17} />{/if}
                <span class="t">{entry.name}</span>
              </button>
            </li>
          {:else}
            <li class="note">Nothing to play in this folder.</li>
          {/each}
        </ul>
      {/if}
    </div>
  {:else}
    <div class="scroll">
      {#if places.mounted.length}
        <div class="label">Mounted shares</div>
        <ul class="rows" aria-label="Mounted shares">
          {#each places.mounted as place (place.path)}
            <li class="row">
              <button class="main" onclick={() => browse([{ name: place.name, location: place.path }])}><HardDrive size={17} /><span class="t">{place.name}</span><span class="d">{place.path}</span></button>
            </li>
          {/each}
        </ul>
      {/if}

      {#if places.servers.length}
        <div class="label">Saved servers</div>
        <ul class="rows" aria-label="Saved servers">
          {#each places.servers as server (server.url)}
            <li class="row">
              <!-- One with a user name needs its password first: those aren't saved. -->
              <button class="main" onclick={() => (server.user ? connect(server.url, server.user) : browse([{ name: server.name, location: server.url }]))}>
                <Server size={17} /><span class="t">{server.name}</span><span class="d">{server.url}</span>
              </button>
              <button class="act" aria-label="Forget {server.name}" data-tip="Forget this server" onclick={() => forget(server.url)}><X size={15} /></button>
            </li>
          {/each}
        </ul>
      {/if}

      <div class="label">On this network</div>
      <ul class="rows" aria-label="On this network">
        {#each places.found as found (found.uri)}
          <li class="row">
            <button class="main" onclick={() => (found.dir ? browse([{ name: found.name, location: found.uri }]) : player.open([found.uri]))}>
              <Wifi size={17} /><span class="t">{found.name}</span><span class="d">{found.uri.split("://")[0].toUpperCase()}</span>
            </button>
          </li>
        {/each}
      </ul>
      <p class="note" role="status">
        {places.found.length ? "Still looking for more…" : "Looking for servers and shared folders on your network…"}
        A share that doesn't show up can be opened by its address with Connect to server.
      </p>
    </div>
  {/if}
</Page>

{#if ui.dialog === "server"}
  <Dialog title="Connect to server" text="A shared folder or media server: smb://, ftp://, sftp://, nfs:// or upnp://." submitLabel="Connect" canSubmit={!!address.trim()} onsubmit={submit}>
    <input bind:this={input} bind:value={address} type="text" placeholder="smb://server/folder" spellcheck="false" autocomplete="off" autocapitalize="off" aria-label="Server address" />
    {#if signsIn}
      <label for="server-user">User name (leave empty if it doesn't need one)</label>
      <input id="server-user" bind:value={user} type="text" spellcheck="false" autocomplete="off" autocapitalize="off" />
      <label for="server-password">Password</label>
      <input id="server-password" bind:value={password} type="password" autocomplete="off" />
    {:else if systemSignIn}
      <p class="hint">{isMac ? "macOS" : "Windows"} asks for the user name and password itself if the share needs them.</p>
    {/if}
    <label class="check"><input type="checkbox" bind:checked={remember} /> Remember this server{signsIn ? " (never its password)" : ""}</label>
  </Dialog>
{/if}

<style>
  .tools {
    display: flex;
    gap: 8px;
    margin: 0 0 12px;
  }
  .hint {
    margin: 12px 0 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .check {
    display: flex !important;
    align-items: center;
    margin-top: 16px !important;
    font-size: 13px !important;
    font-weight: 400 !important;
    color: var(--text) !important;
  }
</style>
