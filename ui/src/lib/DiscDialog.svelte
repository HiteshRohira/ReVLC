<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { Disc3, FileBox, FolderOpen, RefreshCw } from "@lucide/svelte";
  import Dialog from "./Dialog.svelte";
  import { player, type Disc, type DiscKind } from "./player.svelte";
  import { ui } from "./ui.svelte";

  const KINDS: { kind: DiscKind; label: string; hint: string }[] = [
    { kind: "dvd", label: "DVD", hint: "a VIDEO_TS folder or an .iso image" },
    { kind: "bluray", label: "Blu-ray", hint: "a BDMV folder or an .iso image (unencrypted discs only)" },
    { kind: "cd", label: "Audio CD", hint: "an audio CD" },
    { kind: "vcd", label: "VCD", hint: "a Video CD or SVCD" },
  ];

  let discs = $state<Disc[]>([]);
  let kind = $state<DiscKind>("dvd");
  let path = $state<string | null>(null);
  let menus = $state(true);

  const found = $derived(discs.filter((d) => d.kind === kind));
  const hint = $derived(KINDS.find((k) => k.kind === kind)!.hint);
  const hasMenus = $derived(kind === "dvd" || kind === "bluray");
  const name = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;

  async function refresh() {
    discs = await invoke<Disc[]>("disc_list");
    // Start on the kind of the first inserted disc.
    if (!path && discs.length) {
      kind = discs[0].kind;
      path = discs[0].path;
    }
  }

  function setKind(k: DiscKind) {
    kind = k;
    path = discs.find((d) => d.kind === k)?.path ?? null;
  }

  async function chooseFolder() {
    const dir = await openDialog({ directory: true, title: "Choose a disc or disc folder", defaultPath: "/Volumes" });
    if (typeof dir === "string") path = dir;
  }

  async function chooseImage() {
    const file = await openDialog({ multiple: false, title: "Choose a disc image", filters: [{ name: "Disc images", extensions: ["iso", "img", "udf"] }] });
    if (typeof file === "string") path = file;
  }

  function submit() {
    if (!path) return;
    player.openDisc(kind, path, hasMenus && menus);
    ui.dialog = null;
  }

  onMount(refresh);
</script>

<Dialog title="Open disc" text="Play a disc in a drive, {hint}." canSubmit={!!path} onsubmit={submit}>
  <div class="seg" role="radiogroup" aria-label="Disc type">
    {#each KINDS as k}
      <button type="button" role="radio" aria-checked={kind === k.kind} class:on={kind === k.kind} onclick={() => setKind(k.kind)}>{k.label}</button>
    {/each}
  </div>

  <label for="disc-list">Disc <button type="button" class="refresh" data-tip="Look for discs again" aria-label="Look for discs again" onclick={refresh}><RefreshCw size={12} /></button></label>
  <div class="list" id="disc-list" role="radiogroup" aria-label="Disc">
    {#each found as d (d.path)}
      <button type="button" class="opt" role="radio" aria-checked={path === d.path} class:on={path === d.path} onclick={() => (path = d.path)}>
        <Disc3 size={16} /><span>{d.name}</span>
      </button>
    {/each}
    {#if path && !found.some((d) => d.path === path)}
      <button type="button" class="opt on" role="radio" aria-checked="true" data-tip={path}><FileBox size={16} /><span>{name(path)}</span></button>
    {/if}
    {#if !found.length && !path}<div class="none">No {KINDS.find((k) => k.kind === kind)!.label} found. Insert one, or choose a folder or image.</div>{/if}
  </div>
  <div class="choose">
    <button type="button" class="btn" onclick={chooseFolder}><FolderOpen size={15} />Folder…</button>
    {#if kind === "dvd" || kind === "bluray"}<button type="button" class="btn" onclick={chooseImage}><FileBox size={15} />Disc image…</button>{/if}
  </div>

  {#if hasMenus}
    <label class="check"><input type="checkbox" bind:checked={menus} /> Use disc menus</label>
  {/if}
</Dialog>

<style>
  .refresh {
    margin-left: 4px;
    width: 20px;
    height: 20px;
    border-radius: 5px;
    vertical-align: middle;
  }
  .refresh:hover {
    background: var(--hover);
  }
  .list {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-height: 40px;
    max-height: 160px;
    overflow-y: auto;
    padding: 4px;
    border-radius: 9px;
    background: rgb(0 0 0 / 0.3);
  }
  .opt {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 8px 10px;
    border-radius: 7px;
    text-align: left;
    font-size: 13px;
  }
  .opt span {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .opt:hover {
    background: var(--hover);
  }
  .opt.on {
    background: var(--acc-soft);
  }
  .opt.on :global(svg) {
    color: var(--acc);
  }
  .none {
    padding: 10px;
    font-size: 13px;
    color: var(--faint);
  }
  .choose {
    display: flex;
    gap: 8px;
    margin-top: 10px;
  }
  .choose .btn {
    height: 30px;
    font-size: 12.5px;
  }
  .check {
    display: flex !important;
    align-items: center;
    gap: 8px;
    margin-top: 16px !important;
    font-size: 13px !important;
    font-weight: 400 !important;
    color: var(--text) !important;
  }
</style>
