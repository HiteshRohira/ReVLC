<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { AudioLines, Bookmark, BookmarkPlus, Camera, Check, ChevronLeft, ChevronRight, Crop, FileUp, Gauge, ListTree, Moon, Repeat, Rows3, Settings2, X } from "@lucide/svelte";
  import { player, trackName, formatTime, ASPECT_RATIOS, CROPS, SLEEP_PRESETS, SPEEDS, SUBTITLE_EXTENSIONS, type Track } from "./player.svelte";
  import { effectiveBindings, formatBinding } from "./actions";
  import { settings } from "./settings.svelte";

  // "subtitles" is the subtitle button's menu; "settings" is the gear menu.
  let { kind = "settings", onclose }: { kind?: "settings" | "subtitles"; onclose: () => void } = $props();

  type Page = "root" | "speed" | "audio" | "subtitles" | "crop" | "chapters" | "loop" | "bookmarks" | "sleep";
  // svelte-ignore state_referenced_locally
  let page = $state<Page>(kind === "subtitles" ? "subtitles" : "root");

  const s = $derived(player.state);
  const bindings = $derived(effectiveBindings(settings.value?.shortcuts ?? {}));
  const key = (id: string) => formatBinding(bindings[id]);
  const t = $derived(player.tracks);
  const name = (list: Track[], id: number) => trackName(list.find((x) => x.id === id)?.name ?? "Off");
  const speedLabel = $derived(Math.abs(s.rate - 1) < 0.01 ? "Normal" : `${Math.round(s.rate * 100) / 100}×`);
  const cropLabel = $derived(
    player.crop ? `Crop ${player.crop}` : player.aspect ? player.aspect : "Auto",
  );

  async function loadSubtitle() {
    const file = await openDialog({ multiple: false, title: "Add subtitles", filters: [{ name: "Subtitles", extensions: SUBTITLE_EXTENSIONS }] });
    if (typeof file === "string") player.addSubtitle(file);
  }
  const fmtDelay = (ms: number) => `${ms > 0 ? "+" : ""}${(ms / 1000).toFixed(2)} s`;

  const hasChapters = $derived(player.chapters.length > 1 || player.titles.length > 1);
  const chapterLabel = $derived(
    player.chapters.length > 1 && s.chapter >= 0 ? `${s.chapter + 1} of ${player.chapters.length}` : player.titles[s.title]?.name ?? "",
  );
  const loopLabel = $derived(
    s.loopA === null ? "Off" : s.loopB === null ? `From ${formatTime(s.loopA)}` : `${formatTime(s.loopA)}–${formatTime(s.loopB)}`,
  );
  const sleepLabel = $derived(
    s.sleepAfterItem ? "End of item" : s.sleepAt !== null ? `${Math.max(1, Math.ceil((s.sleepAt - Date.now()) / 60000))} min left` : "Off",
  );
  const sleepName = (p: number | "end" | null) => (p === null ? "Off" : p === "end" ? "End of this item" : `${p} minutes`);
  const sleepOn = (p: number | "end" | null) => (p === null ? s.sleepAt === null && !s.sleepAfterItem : p === "end" ? s.sleepAfterItem : false);

  let editing = $state<number | null>(null);
  function rename(time: number, name: string) {
    if (name.trim()) player.renameBookmark(time, name);
    editing = null;
  }
</script>

{#snippet item(label: string, value: string, go: Page, Icon: typeof Gauge)}
  <button class="it" onclick={() => (page = go)}>
    <Icon size={18} /><span class="lab">{label}</span><em>{value}</em><ChevronRight size={15} class="chev" />
  </button>
{/snippet}

{#snippet header(title: string)}
  <button class="it head" onclick={() => (page = "root")}><ChevronLeft size={18} /><span class="lab">{title}</span></button>
  <hr />
{/snippet}

{#snippet choice(label: string, selected: boolean, onclick: () => void, detail?: string)}
  <button class="it choice" class:sel={selected} role="menuitemradio" aria-checked={selected} {onclick}>
    <span class="ck">{#if selected}<Check size={16} />{/if}</span><span class="lab">{label}</span>{#if detail}<em>{detail}</em>{/if}
  </button>
{/snippet}

{#snippet delay(label: string, ms: number, set: (v: number) => void)}
  <div class="delay">
    <div class="row"><span>{label}</span><button class="val tnum" title="Reset" onclick={() => set(0)}>{fmtDelay(ms)}</button></div>
    <input type="range" min="-5000" max="5000" step="50" value={ms} style="--progress: {((ms + 5000) / 10000) * 100}%"
      aria-label={label} oninput={(e) => set(Number(e.currentTarget.value))} />
    <div class="ends"><span>Earlier</span><span>Later</span></div>
  </div>
{/snippet}

<div class="menu" role="menu" aria-label={kind === "subtitles" ? "Subtitles" : "Settings"}>
  {#if page === "root"}
    {@render item("Playback speed", speedLabel, "speed", Gauge)}
    {@render item("Audio", name(t.audio, t.audioCurrent), "audio", AudioLines)}
    {@render item("Crop & aspect", cropLabel, "crop", Crop)}
    <hr />
    {#if hasChapters}{@render item("Chapters", chapterLabel, "chapters", ListTree)}{/if}
    {@render item("A–B loop", loopLabel, "loop", Repeat)}
    {@render item("Bookmarks", player.bookmarks.length ? String(player.bookmarks.length) : "None", "bookmarks", Bookmark)}
    {@render item("Sleep timer", sleepLabel, "sleep", Moon)}
    <hr />
    <button class="it" role="menuitemcheckbox" aria-checked={player.deinterlace} onclick={() => player.setDeinterlace(!player.deinterlace)}>
      <Rows3 size={18} /><span class="lab">Deinterlace</span><span class="tog" class:on={player.deinterlace}></span>
    </button>
    <button class="it" onclick={() => { player.snapshot(); onclose(); }}>
      <Camera size={18} /><span class="lab">Take snapshot</span><em>{key("snapshot")}</em>
    </button>
    <hr />
    <button class="it" onclick={() => { invoke("open_settings"); onclose(); }}>
      <Settings2 size={18} /><span class="lab">More settings…</span><em>{formatBinding("Cmd+,")}</em>
    </button>
  {:else if page === "speed"}
    {@render header("Playback speed")}
    {#each SPEEDS as speed}
      {@render choice(speed === 1 ? "Normal" : `${speed}×`, Math.abs(s.rate - speed) < 0.01, () => player.setRate(speed))}
    {/each}
  {:else if page === "audio"}
    {@render header("Audio")}
    {#each t.audio as track (track.id)}
      {@render choice(trackName(track.name), track.id === t.audioCurrent, () => player.setAudioTrack(track.id))}
    {:else}
      <p class="empty">No audio tracks</p>
    {/each}
    <hr />
    {@render delay("Audio delay", s.audioDelay, (v) => player.setAudioDelay(v))}
  {:else if page === "subtitles"}
    <h5>Subtitles</h5>
    <hr />
    <div class="scroll">
      {#each t.subtitles as track (track.id)}
        {@render choice(trackName(track.name), track.id === t.subtitleCurrent, () => player.setSubtitleTrack(track.id))}
      {:else}
        <p class="empty">No subtitles in this file</p>
      {/each}
    </div>
    <button class="it" onclick={loadSubtitle}><FileUp size={18} /><span class="lab">Add from file…</span></button>
    <hr />
    {@render delay("Subtitle delay", s.subtitleDelay, (v) => player.setSubtitleDelay(v))}
  {:else if page === "chapters"}
    {@render header("Chapters")}
    <div class="scroll tall">
      {#if player.titles.length > 1}
        <h6>Titles</h6>
        {#each player.titles as t, i}
          {@render choice(t.name, i === s.title, () => player.setTitle(i), t.duration > 0 ? formatTime(t.duration) : undefined)}
        {/each}
        {#if player.chapters.length > 1}<hr />{/if}
      {/if}
      {#if player.chapters.length > 1}
        {#if player.titles.length > 1}<h6>Chapters</h6>{/if}
        {#each player.chapters as c, i}
          {@render choice(c.name, i === s.chapter, () => player.setChapter(i), formatTime(c.start))}
        {/each}
      {/if}
    </div>
    {#if player.titles.some((t) => t.menu)}
      <hr />
      <button class="it" onclick={() => { player.navigate("popup"); onclose(); }}><ListTree size={18} /><span class="lab">Disc menu</span><em>{key("discMenu")}</em></button>
    {/if}
  {:else if page === "loop"}
    {@render header("A–B loop")}
    <p class="note">Repeats the part between A and B.</p>
    {#each [["A", s.loopA], ["B", s.loopB]] as const as [point, at]}
      <div class="it static">
        <span class="pt">{point}</span><span class="lab tnum">{at === null ? "Not set" : formatTime(at)}</span>
        <button class="mini" disabled={!s.seekable || (point === "B" && s.loopA !== null && s.time <= s.loopA)}
          onclick={() => (point === "A" ? player.setAbLoop(s.time, s.loopB !== null && s.loopB > s.time ? s.loopB : null) : player.setAbLoop(s.loopA ?? 0, s.time))}>
          Set to {formatTime(s.time)}
        </button>
      </div>
    {/each}
    <hr />
    <button class="it" disabled={s.loopA === null && s.loopB === null} onclick={() => player.setAbLoop(null, null)}>
      <X size={18} /><span class="lab">Clear loop</span><em>{key("abLoop")}</em>
    </button>
  {:else if page === "bookmarks"}
    {@render header("Bookmarks")}
    <button class="it" disabled={!player.active} onclick={() => player.addBookmark()}>
      <BookmarkPlus size={18} /><span class="lab">Add at {formatTime(s.time)}</span><em>{key("addBookmark")}</em>
    </button>
    <hr />
    <div class="scroll">
      {#each player.bookmarks as b (b.time)}
        <div class="bm">
          {#if editing === b.time}
            <!-- svelte-ignore a11y_autofocus -->
            <input class="rename" value={b.name} autofocus aria-label="Bookmark name"
              onkeydown={(e) => { e.stopPropagation(); if (e.key === "Enter") rename(b.time, e.currentTarget.value); if (e.key === "Escape") editing = null; }}
              onblur={(e) => rename(b.time, e.currentTarget.value)} />
          {:else}
            <button class="it" title="Jump here · double-click to rename" onclick={() => player.seek(b.time)} ondblclick={() => (editing = b.time)}>
              <span class="lab">{b.name}</span><em class="tnum">{formatTime(b.time)}</em>
            </button>
          {/if}
          <button class="rm" title="Remove" aria-label="Remove {b.name}" onclick={() => player.removeBookmark(b.time)}><X size={14} /></button>
        </div>
      {:else}
        <p class="empty">No bookmarks in this file</p>
      {/each}
    </div>
  {:else if page === "sleep"}
    {@render header("Sleep timer")}
    {#each SLEEP_PRESETS as p}
      {@render choice(sleepName(p), sleepOn(p), () => player.setSleepTimer(p))}
    {/each}
    {#if s.sleepAt !== null}<p class="note">Pauses in {sleepLabel.replace(" left", "")}.</p>{/if}
  {:else if page === "crop"}
    {@render header("Crop & aspect")}
    <div class="cols">
      <div>
        <h6>Aspect ratio</h6>
        {#each ASPECT_RATIOS as r}{@render choice(r ?? "Auto", player.aspect === r, () => player.setAspect(r))}{/each}
      </div>
      <div>
        <h6>Crop</h6>
        {#each CROPS as c}{@render choice(c ?? "None", player.crop === c, () => player.setCrop(c))}{/each}
      </div>
    </div>
  {/if}
</div>

<style>
  .menu {
    width: 320px;
    max-height: calc(100vh - 140px);
    overflow-y: auto;
    padding: 8px 0;
    border-radius: 14px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 16px 48px rgb(0 0 0 / 0.55);
    font-size: 13.5px;
  }
  .it {
    display: flex;
    align-items: center;
    gap: 12px;
    width: 100%;
    padding: 9px 16px;
    text-align: left;
  }
  .it:hover,
  .it:focus-visible {
    background: var(--hover);
    outline: none;
  }
  .lab {
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  em {
    font-style: normal;
    color: var(--muted);
    max-width: 120px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .it :global(.chev) {
    color: var(--muted);
    flex-shrink: 0;
  }
  .head,
  h5 {
    font-weight: 600;
  }
  h5 {
    margin: 0;
    padding: 9px 16px;
    font-size: inherit;
  }
  .choice .ck {
    width: 18px;
    display: grid;
    place-items: center;
    color: var(--acc);
  }
  .choice.sel {
    font-weight: 600;
  }
  hr {
    border: 0;
    border-top: 1px solid var(--border);
    margin: 6px 0;
  }
  .tog {
    position: relative;
    width: 30px;
    height: 18px;
    border-radius: 9px;
    background: rgb(255 255 255 / 0.22);
    transition: background 0.15s;
  }
  .tog::after {
    content: "";
    position: absolute;
    left: 2px;
    top: 2px;
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.15s;
  }
  .tog.on {
    background: var(--acc);
  }
  .tog.on::after {
    transform: translateX(12px);
    background: var(--on-acc);
  }
  .delay {
    padding: 8px 16px 6px;
  }
  .delay .row {
    display: flex;
    justify-content: space-between;
    margin-bottom: 6px;
  }
  .val {
    color: var(--muted);
  }
  .delay input {
    width: 100%;
  }
  .ends {
    display: flex;
    justify-content: space-between;
    font-size: 11px;
    color: var(--muted);
  }
  .empty {
    margin: 4px 16px 8px;
    color: var(--muted);
  }
  .scroll {
    max-height: 220px;
    overflow-y: auto;
  }
  .tall {
    max-height: 340px;
  }
  .note {
    margin: 2px 16px 8px;
    font-size: 12px;
    color: var(--muted);
  }
  .static:hover {
    background: none;
  }
  .pt {
    width: 18px;
    font-weight: 700;
    text-align: center;
    color: var(--acc);
  }
  .mini {
    height: 26px;
    padding: 0 10px;
    border-radius: 7px;
    background: var(--hover-strong);
    font-size: 12px;
    font-weight: 600;
  }
  .mini:hover:not(:disabled) {
    background: rgb(255 255 255 / 0.18);
  }
  .bm {
    position: relative;
    display: flex;
    align-items: center;
  }
  .bm .it {
    padding-right: 40px;
  }
  .rm {
    position: absolute;
    right: 10px;
    width: 24px;
    height: 24px;
    border-radius: 6px;
    color: var(--muted);
    opacity: 0;
  }
  .bm:hover .rm,
  .rm:focus-visible {
    opacity: 1;
  }
  .rm:hover {
    background: var(--hover-strong);
  }
  .rename {
    flex: 1;
    margin: 4px 44px 4px 12px;
    padding: 5px 8px;
    border-radius: 6px;
    border: 1px solid var(--acc);
    background: rgb(0 0 0 / 0.3);
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .cols {
    display: grid;
    grid-template-columns: 1fr 1fr;
  }
  .cols .it {
    padding: 7px 12px;
    gap: 6px;
  }
  h6 {
    margin: 4px 12px 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
</style>
