<script module lang="ts">
  // Kept while the player has the window, so coming back lands on the same playlist.
  const at = $state({ open: null as string | null });
</script>

<script lang="ts">
  import { onMount } from "svelte";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { ChevronLeft, FileDown, ListMusic, Pencil, Play, Plus, Trash2, X } from "@lucide/svelte";
  import Dialog from "./Dialog.svelte";
  import Page from "./Page.svelte";
  import { player, cleanTitle, PLAYLIST_EXTENSIONS } from "./player.svelte";
  import { ui } from "./ui.svelte";

  /** The playlist being renamed in the dialog; null makes a new one. */
  let renaming = $state<string | null>(null);
  let name = $state("");
  let withQueue = $state(true);
  /** The playlist whose Delete button is waiting for a second click. */
  let deleting = $state<string | null>(null);
  let input = $state<HTMLInputElement>();

  const open = $derived(player.playlists.find((p) => p.id === at.open) ?? null);
  const queued = $derived(player.playlist.items.length);
  const count = (n: number) => `${n} item${n === 1 ? "" : "s"}`;

  onMount(() => {
    player.loadPlaylists();
  });

  $effect(() => {
    if (ui.dialog === "playlist") input?.select();
  });

  function ask(id: string | null) {
    renaming = id;
    name = id ? (player.playlists.find((p) => p.id === id)?.name ?? "") : "";
    withQueue = true;
    ui.dialog = "playlist";
  }

  async function submit() {
    const done = renaming ? await player.renamePlaylist(renaming, name) : await player.createPlaylist(name, withQueue && queued > 0);
    // A name that's taken stays in the box to be changed.
    if (!done) return;
    if (!renaming) at.open = player.playlists[0]?.id ?? null;
    ui.dialog = null;
  }

  async function importFile() {
    const file = await openDialog({ multiple: false, title: "Import a playlist", filters: [{ name: "Playlists", extensions: PLAYLIST_EXTENSIONS }] });
    if (typeof file !== "string") return;
    if (await player.importPlaylist(file)) at.open = player.playlists[0]?.id ?? null;
  }

  async function addFiles(id: string) {
    const files = await openDialog({ multiple: true, title: "Add to playlist" });
    if (!files) return;
    await player.addToPlaylist(id, Array.isArray(files) ? files : [files]);
  }

  async function remove(id: string) {
    if (deleting !== id) {
      deleting = id;
      return;
    }
    deleting = null;
    if (at.open === id) at.open = null;
    await player.deletePlaylist(id);
  }
</script>

<Page title="Playlists">
  {#snippet actions()}
    <button class="btn" onclick={importFile}><FileDown size={16} />Import…</button>
    <button class="btn pri" onclick={() => ask(null)}><Plus size={16} />New playlist</button>
  {/snippet}

  {#if open}
    <div class="scroll">
      <div class="crumbs"><button onclick={() => ((at.open = null), (deleting = null))}><ChevronLeft size={15} />Playlists</button></div>
      <div class="head">
        <div>
          <h2>{open.name}</h2>
          <p>{count(open.items.length)}</p>
        </div>
        <div class="tools">
          <button class="btn pri" disabled={!open.items.length} onclick={() => player.playPlaylist(open.id)}><Play size={15} fill="currentColor" />Play</button>
          <button class="btn" onclick={() => addFiles(open.id)}><Plus size={16} />Add files…</button>
          <button class="btn" onclick={() => ask(open.id)}><Pencil size={15} />Rename</button>
          <button class="btn" class:danger={deleting === open.id} onclick={() => remove(open.id)} onblur={() => (deleting = null)}>
            <Trash2 size={15} />{deleting === open.id ? "Really delete?" : "Delete"}
          </button>
        </div>
      </div>
      {#if open.items.length}
        <ol class="rows" aria-label="Items in {open.name}">
          {#each open.items as item, i (i + item.uri)}
            <li class="row">
              <button class="main" data-tip={item.uri} onclick={() => player.playPlaylist(open.id, i)}>
                <span class="num tnum">{i + 1}</span>
                <span class="t">{cleanTitle(item.title)}</span>
              </button>
              <button class="act" aria-label="Remove {cleanTitle(item.title)} from the playlist" data-tip="Remove" onclick={() => player.removeFromPlaylist(open.id, i)}><X size={15} /></button>
            </li>
          {/each}
        </ol>
      {:else}
        <p class="note">This playlist is empty. Add files to it, or make one from what's playing.</p>
      {/if}
    </div>
  {:else if player.playlists.length}
    <div class="scroll">
      <ul class="rows" aria-label="Playlists">
        {#each player.playlists as list (list.id)}
          <li class="row">
            <button class="main" onclick={() => (at.open = list.id)}>
              <ListMusic size={17} />
              <span class="t">{list.name}</span>
              <span class="d">{count(list.items.length)}</span>
            </button>
            <button class="act" aria-label="Play {list.name}" data-tip="Play" disabled={!list.items.length} onclick={() => player.playPlaylist(list.id)}><Play size={15} fill="currentColor" /></button>
            <button class="act" aria-label="Rename {list.name}" data-tip="Rename" onclick={() => ask(list.id)}><Pencil size={15} /></button>
            <button class="act" class:on={deleting === list.id} aria-label={deleting === list.id ? `Really delete ${list.name}?` : `Delete ${list.name}`}
              data-tip={deleting === list.id ? "Click again to delete" : "Delete"} onclick={() => remove(list.id)} onblur={() => (deleting = null)}><Trash2 size={15} /></button>
          </li>
        {/each}
      </ul>
    </div>
  {:else}
    <div class="blank">
      <ListMusic size={44} strokeWidth={1.4} />
      <h2>Keep what you play together</h2>
      <p>Make a playlist from the queue or from files, or import an M3U, PLS or XSPF file. Playlists stay here between launches.</p>
      <div class="btns">
        <button class="btn pri" onclick={() => ask(null)}><Plus size={16} />New playlist</button>
        <button class="btn" onclick={importFile}><FileDown size={16} />Import…</button>
      </div>
    </div>
  {/if}
</Page>

{#if ui.dialog === "playlist"}
  <Dialog title={renaming ? "Rename playlist" : "New playlist"} text={renaming ? "Give it a new name." : "Playlists stay here between launches."} submitLabel={renaming ? "Rename" : "Create"} canSubmit={!!name.trim()} onsubmit={submit}>
    <input bind:this={input} bind:value={name} type="text" placeholder="Name" maxlength="120" spellcheck="false" autocomplete="off" aria-label="Playlist name" />
    {#if !renaming && queued > 0}
      <label class="check"><input type="checkbox" bind:checked={withQueue} /> Add the {count(queued)} in the queue</label>
    {/if}
  </Dialog>
{/if}

<style>
  .head {
    display: flex;
    align-items: flex-end;
    justify-content: space-between;
    flex-wrap: wrap;
    gap: 14px;
    margin-bottom: 16px;
  }
  .head h2 {
    margin: 0 0 2px;
    font-size: 24px;
  }
  .head p {
    margin: 0;
    font-size: 12.5px;
    color: var(--muted);
  }
  .tools {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
  }
  .check {
    display: flex !important;
    align-items: center;
    margin-top: 14px !important;
    font-size: 13px !important;
    font-weight: 400 !important;
    color: var(--text) !important;
  }
</style>
