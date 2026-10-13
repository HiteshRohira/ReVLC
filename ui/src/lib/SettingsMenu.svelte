<script lang="ts">
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { AudioLines, Camera, Captions, Check, ChevronLeft, ChevronRight, Crop, FileUp, Gauge, Rows3 } from "@lucide/svelte";
  import { player, trackName, ASPECT_RATIOS, CROPS, SPEEDS, SUBTITLE_EXTENSIONS, type Track } from "./player.svelte";

  let { onclose }: { onclose: () => void } = $props();

  type Page = "root" | "speed" | "audio" | "subtitles" | "crop";
  let page = $state<Page>("root");

  const s = $derived(player.state);
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

<div class="menu" role="menu" aria-label="Settings">
  {#if page === "root"}
    {@render item("Playback speed", speedLabel, "speed", Gauge)}
    {@render item("Audio", name(t.audio, t.audioCurrent), "audio", AudioLines)}
    {@render item("Subtitles", name(t.subtitles, t.subtitleCurrent), "subtitles", Captions)}
    {@render item("Crop & aspect", cropLabel, "crop", Crop)}
    <hr />
    <button class="it" role="menuitemcheckbox" aria-checked={player.deinterlace} onclick={() => player.setDeinterlace(!player.deinterlace)}>
      <Rows3 size={18} /><span class="lab">Deinterlace</span><span class="tog" class:on={player.deinterlace}></span>
    </button>
    <button class="it" onclick={() => { player.snapshot(); onclose(); }}>
      <Camera size={18} /><span class="lab">Take snapshot</span><em>⇧S</em>
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
    {@render header("Subtitles")}
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
  .head {
    font-weight: 600;
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
