<script lang="ts">
  import { player, formatTime, chapterAt, genericChapter } from "./player.svelte";

  const s = $derived(player.state);
  const length = $derived(s.length);
  const enabled = $derived(player.active && s.seekable && length > 0);

  let el: HTMLDivElement;
  let hover = $state<number | null>(null);
  let drag = $state<number | null>(null);
  const position = $derived(drag ?? s.time);

  // Chapters as segments; a single segment when the media has none.
  const segments = $derived(
    player.chapters.length > 1 && length > 0
      ? player.chapters.map((c, i) => ({
          start: c.start,
          end: player.chapters[i + 1]?.start ?? length,
        }))
      : [{ start: 0, end: Math.max(length, 1) }],
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
    if (drag !== null) drag = hover;
  }
  function onUp() {
    if (drag !== null) player.seek(drag);
    drag = null;
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
  {#if hover !== null && enabled}
    {@const ch = player.chapters.length > 1 ? chapterAt(player.chapters, hover) : null}
    <div class="tip" style="left: clamp(60px, {(hover / length) * 100}%, calc(100% - 60px))">
      <span class="tnum">{formatTime(hover)}</span>{#if ch && !genericChapter(ch.name)}<span class="ch"> · {ch.name}</span>{/if}
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
  }
  .ch {
    font-weight: 400;
    color: var(--muted);
  }
</style>
