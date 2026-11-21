<script lang="ts">
  import { untrack } from "svelte";
  import { fly } from "svelte/transition";
  import { player, formatTime, chapterAt, genericChapter, thumbUrl } from "./player.svelte";
  import { settings } from "./settings.svelte";

  const s = $derived(player.state);
  const length = $derived(s.length);
  const enabled = $derived(player.active && s.seekable && length > 0);

  // Make preview frames once there's a seekable local video to take them
  // from: for the selected video track, so picking another restarts them.
  // A string, so unrelated changes don't restart them.
  const wantFrames = $derived(
    player.current && enabled && settings.value?.seekbarPreview !== false
      ? `${player.current.uri}\n${player.tracks.videoCurrent}`
      : null,
  );
  /** Whether this video gets previews: "pending" until the player says, so the hover box can show it's coming. */
  let previews = $state<"pending" | "on" | "off">("off");
  $effect(() => {
    // Another video or track, or none: the close-up strip and the previews
    // belong to the last one, and making them stops (previews turned off,
    // playback stopped, Home).
    untrack(() => {
      fineOpen = false;
      pull = 0;
      fineAt = null;
      player.cancelScrub();
      // Only local files can have previews.
      previews = wantFrames === null || wantFrames.includes("://") ? "off" : "pending";
    });
    if (wantFrames === null) return;
    // The video track may not be known yet when the length is, so ask again a few times.
    let timer: ReturnType<typeof setTimeout>;
    const ask = async (left: number) => {
      await player.prepareScrub();
      if (player.scrub) previews = "on";
      else if (left > 0) timer = setTimeout(() => ask(left - 1), 1000);
      else previews = "off";
    };
    // Not while the video is starting: decoding previews then would hold it up.
    timer = setTimeout(() => ask(3), 2500);
    return () => {
      clearTimeout(timer);
      // Leaving the player (Home) stops them too.
      untrack(() => player.cancelScrub());
    };
  });

  // The preview frame under the pointer (the strip shows them while it's open).
  const hoverK = $derived.by(() => (hover === null || !player.scrub ? null : Math.round(hover / player.scrub.step)));
  const frame = $derived.by(() => (fineOpen || hover === null ? null : player.nearestScrubFrame(hover, length)));
  // While scrubbing, the video shows the frame of the chosen spot, as on
  // YouTube; the real seek comes when the user lets go or plays.
  $effect(() => {
    const scrubbing = drag !== null || fineOpen;
    const shown = scrubbing ? player.nearestScrubFrame(position, length) : null;
    untrack(() => (player.scrubShown = shown));
    return () => untrack(() => (player.scrubShown = null));
  });
  // Have the frames under the pointer made next.
  $effect(() => {
    if (hoverK !== null && !player.scrub?.ready.has(hoverK)) untrack(() => player.focusScrub(hoverK - 1, hoverK + 1));
  });

  // Dragging up off the bar opens a strip of close-up frames, like YouTube's
  // precise seeking. It stays open, with the video paused, while the user
  // scrubs either the bar or the strip (a zoomed-in stretch of the bar). It
  // closes, jumping to the chosen spot, from its close button or on play.
  const FINE_UP = 40; // px above the bar where fine scrubbing starts
  const TILE = 156; // px per close-up frame, gap included
  const fineStep = $derived(player.scrub?.step ?? 10_000);
  let fineOpen = $state(false);
  /** How far the strip has been pulled up, 0 to 1, while dragging up towards opening it. */
  let pull = $state(0);
  const stripShown = $derived(fineOpen || pull > 0);
  /** The chosen spot while the strip is open. */
  let fineT = $state(0);
  /** Where a drag that opened the strip started: then the pointer's sideways travel moves the spot. */
  let fineAt = $state<{ x: number; t: number } | null>(null);
  /** Where dragging the strip itself started: it pans like a scroll. */
  let stripDrag = $state<{ x: number; t: number } | null>(null);
  /** Briefly true while the strip slides to a clicked frame. */
  let gliding = $state(false);
  let glideTimer: ReturnType<typeof setTimeout> | undefined;
  let seenPaused = false;
  let barW = $state(0);
  let asked = -1000;
  const half = $derived((barW / 2) * (fineStep / TILE)); // ms from the middle to an edge
  const fineFrames = $derived.by(() => {
    if (!stripShown) return [];
    const first = Math.floor((position - half) / fineStep);
    const last = Math.ceil((position + half) / fineStep);
    const out: { k: number; left: number }[] = [];
    for (let k = Math.max(0, first); k <= Math.min(Math.floor(length / fineStep), last); k++) {
      out.push({ k, left: barW / 2 + ((k * fineStep - position) / fineStep) * TILE });
    }
    return out;
  });
  // Ask for the frames around the pointer as it moves along.
  $effect(() => {
    if (!stripShown) {
      asked = -1000;
      return;
    }
    const mid = Math.round(position / fineStep);
    if (Math.abs(mid - asked) < 2) return;
    asked = mid;
    const reach = Math.ceil(half / fineStep) + 2;
    untrack(() => player.focusScrub(mid - reach, mid + reach));
  });

  let el: HTMLDivElement;
  let hover = $state<number | null>(null);
  let drag = $state<number | null>(null);
  const position = $derived(drag ?? (fineOpen ? fineT : s.time));

  function openFine() {
    if (fineOpen) return;
    fineOpen = true;
    // Open now, so no longer pulled: closing must hide it altogether.
    pull = 0;
    fineT = drag ?? s.time;
    seenPaused = false;
    if (player.playing) player.togglePause();
  }
  /** Leaves precise seeking for the chosen spot, playing from there. */
  function closeFine(resume: boolean) {
    if (!fineOpen) return;
    const t = fineT;
    fineOpen = false;
    pull = 0;
    fineAt = null;
    stripDrag = null;
    player.seek(t);
    if (resume && !player.playing) player.togglePause();
  }
  // A click on the video closes the strip too (see PlayerView and App).
  $effect(() => {
    player.closeStrip = fineOpen ? () => closeFine(true) : null;
    return () => (player.closeStrip = null);
  });
  // Playing again, by any means, takes the chosen spot with it.
  $effect(() => {
    if (!fineOpen) return;
    const playing = player.playing;
    untrack(() => {
      if (!playing) seenPaused = true;
      else if (seenPaused) closeFine(false);
    });
  });

  // Chapters as segments; a single segment when the media has none.
  const segments = $derived(
    player.chapters.length > 1 && length > 0 && settings.value?.seekbarChapters !== false
      ? player.chapters.map((c, i) => ({
          start: c.start,
          end: player.chapters[i + 1]?.start ?? length,
        }))
      : [{ start: 0, end: Math.max(length, 1) }],
  );

  const pct = (ms: number) => `${Math.min(100, Math.max(0, (ms / length) * 100))}%`;
  // A bookmark near the pointer (within ~1% of the bar), for the tooltip.
  const nearBookmark = $derived(
    hover === null ? null : (player.bookmarks.find((b) => Math.abs(b.time - hover!) < length / 100) ?? null),
  );

  function msAt(e: PointerEvent) {
    const r = el.getBoundingClientRect();
    return Math.min(1, Math.max(0, (e.clientX - r.left) / r.width)) * length;
  }
  function fill(seg: { start: number; end: number }, ms: number) {
    return Math.min(100, Math.max(0, ((ms - seg.start) / (seg.end - seg.start)) * 100));
  }
  function onDown(e: PointerEvent) {
    if (!enabled) return;
    el.setPointerCapture(e.pointerId);
    drag = msAt(e);
  }
  function onMove(e: PointerEvent) {
    if (!enabled) return;
    hover = msAt(e);
    if (drag === null) return;
    const up = el.getBoundingClientRect().top - e.clientY;
    // The strip follows the pointer up until it opens.
    if (!fineOpen) pull = player.scrub ? Math.min(1, Math.max(0, (up - 6) / FINE_UP)) : 0;
    // Fine scrubbing starts above FINE_UP and ends a little below it.
    if (fineAt === null && up > FINE_UP && player.scrub) {
      fineAt = { x: e.clientX, t: drag };
      openFine();
    } else if (fineAt !== null && up < FINE_UP - 15) fineAt = null;
    drag = fineAt
      ? Math.min(length, Math.max(0, fineAt.t + ((e.clientX - fineAt.x) * fineStep) / TILE))
      : hover;
  }
  function onUp() {
    if (drag !== null) {
      if (fineOpen) fineT = drag;
      else player.seek(drag);
    }
    drag = null;
    fineAt = null;
    // Not pulled far enough: it slides back down.
    if (!fineOpen) pull = 0;
  }
  const clamp = (ms: number) => Math.min(length, Math.max(0, ms));
  function onStripDown(e: PointerEvent) {
    e.stopPropagation();
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    stripDrag = { x: e.clientX, t: fineT };
  }
  function onStripMove(e: PointerEvent) {
    e.stopPropagation();
    if (stripDrag) fineT = clamp(stripDrag.t - ((e.clientX - stripDrag.x) * fineStep) / TILE);
  }
  function onStripUp(e: PointerEvent) {
    e.stopPropagation();
    // A click rather than a drag: the clicked frame slides to the middle.
    if (stripDrag && Math.abs(e.clientX - stripDrag.x) < 4) {
      const x = e.clientX - (e.currentTarget as HTMLElement).getBoundingClientRect().left;
      const hit = fineFrames.reduce<{ k: number; left: number } | null>(
        (best, f) => (Math.abs(f.left - x) < Math.abs((best?.left ?? Infinity) - x) ? f : best),
        null,
      );
      if (hit && Math.abs(hit.left - x) <= TILE / 2) {
        gliding = true;
        clearTimeout(glideTimer);
        glideTimer = setTimeout(() => (gliding = false), 220);
        fineT = clamp(hit.k * fineStep);
      }
    }
    stripDrag = null;
  }
  function onKey(e: KeyboardEvent) {
    if (e.key === "ArrowLeft" || e.key === "ArrowRight") e.stopPropagation();
    if (e.key === "ArrowLeft") player.seekBy(-5000);
    if (e.key === "ArrowRight") player.seekBy(5000);
  }
</script>

<div
  class="seek"
  class:enabled
  class:active={drag !== null}
  bind:this={el}
  bind:clientWidth={barW}
  role="slider"
  tabindex="0"
  aria-label="Seek"
  aria-valuemin={0}
  aria-valuemax={length}
  aria-valuenow={position}
  aria-valuetext={formatTime(position)}
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointerleave={() => (hover = null)}
  onkeydown={onKey}
>
  {#if stripShown}
    {@const p = fineOpen ? 1 : pull}
    <div
      class="strip"
      class:panning={stripDrag !== null}
      class:gliding
      class:following={!fineOpen && drag !== null}
      class:peek={!fineOpen}
      style="opacity: {p}; transform: translateY({(1 - p) * 28}px) scaleY({0.55 + 0.45 * p})"
      out:fly={{ y: 28, duration: 160 }}
      role="presentation" onpointerdown={onStripDown} onpointermove={onStripMove} onpointerup={onStripUp}>
      {#each fineFrames as f (f.k)}
        {@const src = player.scrubFrame(f.k)}
        <div class="tile" style="left: {f.left}px">
          {#if src}<img src={thumbUrl(src, player.scrubVersion)} alt="" draggable="false" />{/if}
        </div>
      {/each}
      <div class="mid"></div>
      <div class="when tnum">{formatTime(position)}</div>
      <button class="close" aria-label="Close precise seeking" onpointerdown={(e) => e.stopPropagation()} onclick={() => closeFine(true)}>
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true"><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
      </button>
    </div>
  {/if}
  {#if hover !== null && enabled && fineAt === null && pull < 0.3}
    {@const ch = player.chapters.length > 1 ? chapterAt(player.chapters, hover) : null}
    <!-- No preview box while the zoomed strip is open: it shows the frames. -->
    {@const boxed = previews !== "off" && !fineOpen}
    <div class="tip" class:framed={boxed} style="left: clamp({boxed ? 92 : 60}px, {(hover / length) * 100}%, calc(100% - {boxed ? 92 : 60}px))">
      {#if frame}
        <img src={thumbUrl(frame, player.scrubVersion)} alt="" draggable="false" />
      {:else if boxed}
        <!-- The frame here isn't made yet. -->
        <div class="making" role="status" aria-label="Making preview"><div class="ring"></div></div>
      {/if}
      <span class="tnum">{formatTime(hover)}</span>{#if nearBookmark}<span class="ch"> · {nearBookmark.name}</span>{:else if ch && !genericChapter(ch.name)}<span class="ch"> · {ch.name}</span>{/if}
    </div>
  {/if}
  <div class="segs">
    {#each segments as seg}
      <div class="seg" style="flex: {seg.end - seg.start}">
        {#if hover !== null && enabled}<i class="hover" style="width: {fill(seg, hover)}%"></i>{/if}
        <i class="done" style="width: {fill(seg, position)}%"></i>
      </div>
    {/each}
  </div>
  {#if enabled}
    {#if s.loopA !== null}
      <div class="ab" class:open={s.loopB === null} style="left: {pct(s.loopA)}; width: {s.loopB !== null ? `calc(${pct(s.loopB)} - ${pct(s.loopA)})` : '2px'}"></div>
    {/if}
    {#each player.bookmarks as b (b.time)}<div class="bm" style="left: {pct(b.time)}"></div>{/each}
  {/if}
  {#if enabled}<div class="knob" style="left: {(position / length) * 100}%"></div>{/if}
</div>

<style>
  .seek {
    position: relative;
    height: 22px;
    display: flex;
    align-items: center;
    outline: none;
  }
  .enabled {
    cursor: pointer;
  }
  .segs {
    display: flex;
    gap: 3px;
    width: 100%;
    height: 4px;
    transition: height 0.12s;
  }
  .enabled:hover .segs,
  .active .segs {
    height: 6px;
  }
  .seg {
    position: relative;
    min-width: 2px;
    height: 100%;
    border-radius: 2px;
    background: rgb(255 255 255 / 0.25);
    overflow: hidden;
  }
  .seg i {
    position: absolute;
    inset: 0 auto 0 0;
  }
  .hover {
    background: rgb(255 255 255 / 0.35);
  }
  .done {
    background: var(--acc);
  }
  /* The A–B loop range, drawn just above the bar. */
  .ab {
    position: absolute;
    top: 50%;
    height: 12px;
    margin-top: -6px;
    border-radius: 3px;
    border: 1.5px solid var(--acc);
    box-sizing: border-box;
    pointer-events: none;
  }
  .ab.open {
    border-width: 0 0 0 2px;
    border-radius: 0;
  }
  .bm {
    position: absolute;
    top: 50%;
    width: 3px;
    height: 10px;
    margin: -5px 0 0 -1.5px;
    border-radius: 1.5px;
    background: #fff;
    box-shadow: 0 0 2px rgb(0 0 0 / 0.6);
    pointer-events: none;
  }
  .knob {
    position: absolute;
    top: 50%;
    width: 14px;
    height: 14px;
    margin: -7px 0 0 -7px;
    border-radius: 50%;
    background: var(--acc);
    box-shadow: 0 1px 4px rgb(0 0 0 / 0.5);
    transform: scale(0);
    transition: transform 0.12s;
  }
  .enabled:hover .knob,
  .active .knob,
  .seek:focus-visible .knob {
    transform: scale(1);
  }
  .tip {
    position: absolute;
    bottom: 24px;
    transform: translateX(-50%);
    padding: 4px 9px;
    border-radius: 7px;
    background: rgb(20 20 22 / 0.95);
    font-size: 12px;
    font-weight: 600;
    white-space: nowrap;
    pointer-events: none;
    z-index: 3;
  }
  /* The close-up frames shown while dragging up, sliding past a fixed middle. */
  .strip {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 34px;
    height: 84px;
    overflow: hidden;
    border-radius: 8px;
    background: rgb(10 10 12 / 0.92);
    cursor: grab;
    touch-action: none;
    transform-origin: 50% 100%;
    /* Settles into place when it opens; follows the pointer while pulled. */
    transition:
      opacity 0.18s ease-out,
      transform 0.18s ease-out;
  }
  .strip.following {
    transition: none;
  }
  .strip.peek {
    pointer-events: none;
  }
  .strip.panning {
    cursor: grabbing;
  }
  .strip.gliding .tile {
    transition: left 0.2s ease;
  }
  .close {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 22px;
    height: 22px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: 50%;
    background: rgb(0 0 0 / 0.7);
    color: #fff;
    cursor: pointer;
  }
  .close:hover {
    background: rgb(255 255 255 / 0.25);
  }
  .tile {
    position: absolute;
    top: 0;
    width: 148px;
    height: 84px;
    margin-left: -74px;
    background: rgb(255 255 255 / 0.06);
  }
  .tile img {
    display: block;
    width: 100%;
    height: 100%;
    object-fit: cover;
  }
  .mid {
    position: absolute;
    top: 0;
    bottom: 0;
    left: 50%;
    width: 2px;
    margin-left: -1px;
    background: #fff;
    box-shadow: 0 0 3px rgb(0 0 0 / 0.7);
  }
  .when {
    position: absolute;
    left: 50%;
    bottom: 4px;
    transform: translateX(-50%);
    padding: 1px 7px;
    border-radius: 5px;
    background: rgb(0 0 0 / 0.75);
    font-size: 12px;
    font-weight: 600;
  }
  .tip.framed {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 5px;
    padding: 4px 4px 5px;
  }
  .tip img {
    display: block;
    width: 168px;
    max-height: 120px;
    object-fit: contain;
    border-radius: 5px;
    background: #000;
  }
  .making {
    display: grid;
    place-items: center;
    width: 168px;
    aspect-ratio: 16 / 9;
    border-radius: 5px;
    background: #000;
  }
  .ring {
    width: 20px;
    height: 20px;
    border-radius: 50%;
    border: 2px solid rgb(255 255 255 / 0.2);
    border-top-color: #fff;
    animation: spin 0.8s linear infinite;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .ch {
    font-weight: 400;
    color: var(--muted);
  }
</style>
