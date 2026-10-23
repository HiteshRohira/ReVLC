<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { X } from "@lucide/svelte";
  import { player, type MediaInfo, type Stats, type TrackInfo } from "./player.svelte";
  import { ui } from "./ui.svelte";

  const TABS = [["general", "General"], ["codec", "Codec"], ["stats", "Statistics"]] as const;

  let info = $state<MediaInfo | null>(null);
  let stats = $state<Stats | null>(null);
  let failed = $state(false);

  const close = () => (ui.dialog = null);

  // Reread when the item changes, or when its tracks do (they appear as it opens).
  const uri = $derived(player.current?.uri);
  $effect(() => {
    uri;
    player.tracks;
    invoke<MediaInfo | null>("media_info").then((i) => ((info = i), (failed = false)), () => (failed = true));
  });

  // Statistics refresh live while their tab shows.
  $effect(() => {
    if (ui.infoTab !== "stats") return;
    const load = () => invoke<Stats | null>("media_stats").then((s) => (stats = s), () => (stats = null));
    load();
    const timer = setInterval(load, 1000);
    return () => clearInterval(timer);
  });

  const KINDS: Record<TrackInfo["kind"], string> = { video: "Video", audio: "Audio", text: "Subtitles", unknown: "Other" };
  const tracks = $derived(
    [...(info?.tracks ?? [])].sort((a, b) => ["video", "audio", "text", "unknown"].indexOf(a.kind) - ["video", "audio", "text", "unknown"].indexOf(b.kind)),
  );

  function details(t: TrackInfo): [string, string][] {
    const rows: [string, string | null | false][] = [
      ["Codec", t.codecName ? `${t.codecName} (${t.codec})` : t.codec],
      ["Language", t.language],
      ["Description", t.description],
      ["Resolution", t.width > 0 && `${t.width} × ${t.height}`],
      ["Frame rate", t.fps > 0 && `${Math.round(t.fps * 1000) / 1000} fps`],
      ["Pixel aspect ratio", t.sar !== "1:1" && t.sar],
      ["Channels", t.channels > 0 && channels(t.channels)],
      ["Sample rate", t.rate > 0 && `${t.rate.toLocaleString()} Hz`],
      ["Bitrate", t.bitrate > 0 && `${Math.round(t.bitrate / 1000).toLocaleString()} kb/s`],
      ["Encoding", t.encoding],
    ];
    return rows.filter((r): r is [string, string] => !!r[1]);
  }
  const channels = (n: number) => ({ 1: "Mono", 2: "Stereo", 6: "5.1", 8: "7.1" })[n] ?? `${n}`;

  const kb = (bytes: number) => `${Math.round(bytes / 1024).toLocaleString()} KiB`;
  const rate = (kbps: number) => `${Math.round(kbps).toLocaleString()} kb/s`;
  const n = (v: number) => v.toLocaleString();
  const STAT_GROUPS = $derived<[string, [string, string][]][]>(
    stats
      ? [
          ["Input", [["Read from media", kb(stats.readBytes)], ["Input bitrate", rate(stats.inputBitrate)]]],
          ["Demuxer", [
            ["Demuxed", kb(stats.demuxReadBytes)], ["Content bitrate", rate(stats.demuxBitrate)],
            ["Corrupted", n(stats.demuxCorrupted)], ["Discontinuities", n(stats.demuxDiscontinuity)],
          ]],
          ["Video", [["Decoded frames", n(stats.decodedVideo)], ["Displayed frames", n(stats.displayedPictures)], ["Lost frames", n(stats.lostPictures)]]],
          ["Audio", [["Decoded blocks", n(stats.decodedAudio)], ["Played buffers", n(stats.playedAbuffers)], ["Lost buffers", n(stats.lostAbuffers)]]],
        ]
      : [],
  );
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div class="dialog" role="dialog" aria-modal="true" aria-label="Media information">
    <header>
      <h2>Media information</h2>
      <button class="icon" data-tip="Close" aria-label="Close" onclick={close}><X size={18} /></button>
    </header>
    <div class="tabs" role="tablist">
      {#each TABS as [id, label]}
        <button role="tab" aria-selected={ui.infoTab === id} class:on={ui.infoTab === id} onclick={() => (ui.infoTab = id)}>{label}</button>
      {/each}
    </div>
    <div class="body">
      {#if failed}
        <p class="muted">Media information isn't available.</p>
      {:else if !info}
        <p class="muted">Nothing is playing.</p>
      {:else if ui.infoTab === "general"}
        <dl>
          {#each info.meta as [field, value]}<dt>{field}</dt><dd>{value}</dd>{/each}
          <dt>Location</dt><dd class="loc">{info.location}</dd>
        </dl>
      {:else if ui.infoTab === "codec"}
        {#each tracks as t (t.kind + t.id)}
          <h3>{KINDS[t.kind]} <span>track {t.id}</span></h3>
          <dl>{#each details(t) as [k, v]}<dt>{k}</dt><dd>{v}</dd>{/each}</dl>
        {:else}
          <p class="muted">No tracks yet.</p>
        {/each}
      {:else if !stats}
        <p class="muted">No statistics yet.</p>
      {:else}
        {#each STAT_GROUPS as [group, rows]}
          <h3>{group}</h3>
          <dl>{#each rows as [k, v]}<dt>{k}</dt><dd class="tnum">{v}</dd>{/each}</dl>
        {/each}
      {/if}
    </div>
  </div>
</div>

<style>
  .backdrop {
    position: fixed;
    inset: 0;
    display: grid;
    place-items: center;
    background: rgb(0 0 0 / 0.5);
    z-index: 50;
  }
  .dialog {
    display: flex;
    flex-direction: column;
    width: min(560px, calc(100vw - 48px));
    max-height: min(620px, calc(100vh - 80px));
    border-radius: 14px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 20px 60px rgb(0 0 0 / 0.6);
    font-size: 13px;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 14px 12px 4px 20px;
  }
  h2 {
    margin: 0;
    font-size: 16px;
  }
  .icon {
    width: 30px;
    height: 30px;
    border-radius: 8px;
  }
  .icon:hover {
    background: var(--hover-strong);
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin: 8px 20px 4px;
    padding: 2px;
    border-radius: 9px;
    background: rgb(0 0 0 / 0.3);
  }
  .tabs button {
    flex: 1;
    height: 28px;
    border-radius: 7px;
    font-size: 12.5px;
    color: var(--muted);
  }
  .tabs button.on {
    background: var(--hover-strong);
    color: var(--text);
    font-weight: 600;
  }
  .body {
    overflow-y: auto;
    padding: 10px 20px 20px;
  }
  h3 {
    margin: 14px 0 6px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  h3 span {
    text-transform: none;
    font-weight: 400;
    letter-spacing: 0;
  }
  dl {
    display: grid;
    grid-template-columns: 150px 1fr;
    gap: 6px 14px;
    margin: 0;
  }
  dt {
    color: var(--muted);
  }
  dd {
    margin: 0;
    overflow-wrap: anywhere;
    user-select: text;
  }
  .loc {
    font-size: 12px;
  }
  .muted {
    color: var(--muted);
  }
</style>
