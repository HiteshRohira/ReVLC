<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { RotateCcw, X } from "@lucide/svelte";
  import Select from "./Select.svelte";
  import { player, DEFAULT_EFFECTS, type Adjust, type Equalizer, type Logo, type Marquee } from "./player.svelte";
  import { ui } from "./ui.svelte";

  const TABS = [["equalizer", "Equaliser"], ["colour", "Colour"], ["overlays", "Overlays"]] as const;
  let tab = $state<(typeof TABS)[number][0]>("equalizer");

  let presets = $state<string[]>([]);
  let bands = $state<number[]>([]);
  invoke<{ presets: string[]; bands: number[] }>("equalizer_info").then((info) => ({ presets, bands } = info), () => {});

  const fx = $derived(player.effects);
  const eq = $derived(fx.equalizer);
  const freq = (hz: number) => (hz >= 1000 ? `${Math.round(hz / 100) / 10}k` : `${Math.round(hz)}`).replace(".0k", "k");
  const db = (v: number) => `${v > 0 ? "+" : ""}${v.toFixed(1)} dB`;

  // Changes show at once; the backend's reply confirms them.
  function setEq(patch: Partial<Equalizer>) {
    player.effects.equalizer = { ...eq, enabled: true, ...patch };
    player.setEqualizer(player.effects.equalizer);
  }
  function setAdjust(patch: Partial<Adjust>) {
    player.effects.adjust = { ...fx.adjust, ...patch };
    player.setAdjust(player.effects.adjust);
  }
  function setLogo(patch: Partial<Logo>) {
    player.effects.logo = { ...fx.logo, ...patch };
    player.setLogo(player.effects.logo);
  }
  function setMarquee(patch: Partial<Marquee>) {
    player.effects.marquee = { ...fx.marquee, ...patch };
    player.setMarquee(player.effects.marquee);
  }

  // [key, label, min, max, step, format]. Gamma's slider is logarithmic (see below).
  const ADJUST: [Exclude<keyof Adjust, "enabled">, string, number, number, number, (v: number) => string][] = [
    ["brightness", "Brightness", 0, 2, 0.01, (v) => v.toFixed(2)],
    ["contrast", "Contrast", 0, 2, 0.01, (v) => v.toFixed(2)],
    ["hue", "Hue", -180, 180, 1, (v) => `${Math.round(v)}°`],
    ["saturation", "Saturation", 0, 3, 0.01, (v) => v.toFixed(2)],
    ["gamma", "Gamma", -2, 1, 0.01, (v) => v.toFixed(2)],
  ];
  // Gamma runs from 0.01 to 10 with 1 in the middle of most use, so the slider moves its exponent.
  const toSlider = (key: string, v: number) => (key === "gamma" ? Math.log10(v) : v);
  const fromSlider = (key: string, v: number) => (key === "gamma" ? Math.round(10 ** v * 100) / 100 : v);
  const adjustChanged = $derived(ADJUST.some(([k]) => Math.abs(fx.adjust[k] - DEFAULT_EFFECTS.adjust[k]) > 1e-3));

  const POSITIONS: [number, string][] = [
    [5, "Top left"], [4, "Top"], [6, "Top right"], [1, "Left"], [0, "Centre"], [2, "Right"], [9, "Bottom left"], [8, "Bottom"], [10, "Bottom right"],
  ];
  const COLORS: [number, string][] = [
    [0xffffff, "White"], [0x000000, "Black"], [0xffff00, "Yellow"], [0xff0000, "Red"], [0x00ff00, "Lime"], [0x00ffff, "Aqua"], [0x0000ff, "Blue"], [0xff00ff, "Fuchsia"], [0x808080, "Gray"],
  ];
  const pct = (n: number) => `${Math.round((n / 255) * 100)}%`;

  async function chooseLogo() {
    const file = await openDialog({ multiple: false, title: "Choose a logo", filters: [{ name: "Images", extensions: ["png", "jpg", "jpeg", "gif", "bmp", "tga"] }] });
    if (typeof file === "string") setLogo({ file, enabled: true });
  }
</script>

{#snippet sw(on: boolean, label: string, set: (on: boolean) => void)}
  <button class="switch" class:on role="switch" aria-checked={on} aria-label={label} onclick={() => set(!on)}></button>
{/snippet}

{#snippet slider(label: string, value: number, min: number, max: number, step: number, text: string, set: (v: number) => void, disabled = false)}
  <div class="slider" class:off={disabled}>
    <div class="top"><span>{label}</span><span class="tnum muted">{text}</span></div>
    <input type="range" {min} {max} {step} {value} aria-label={label} style="--progress: {((value - min) / (max - min)) * 100}%"
      oninput={(e) => set(Number(e.currentTarget.value))} />
  </div>
{/snippet}

<aside class="effects" aria-label="Effects and filters">
  <header>
    <h2>Effects and filters</h2>
    <button class="icon" data-tip="Close" aria-label="Close" onclick={() => (ui.effectsOpen = false)}><X size={18} /></button>
  </header>
  <div class="tabs" role="tablist">
    {#each TABS as [id, label]}
      <button role="tab" aria-selected={tab === id} class:on={tab === id} onclick={() => (tab = id)}>{label}</button>
    {/each}
  </div>

  <div class="body">
    {#if tab === "equalizer"}
      <div class="line"><span>Equaliser</span>{@render sw(eq.enabled, "Equaliser", (on) => setEq({ enabled: on }))}</div>
      <div class="line">
        <span>Preset</span>
        <Select value={eq.preset ?? -1}
          options={[...(eq.preset === null ? [[-1, "Custom"] as [number, string]] : []), ...presets.map((name, i) => [i, name] as [number, string])]}
          onchange={(i) => Number(i) >= 0 && player.setEqualizerPreset(Number(i))} />
      </div>
      <div class="eq" class:off={!eq.enabled}>
        <div class="band pre">
          <span class="tnum val">{eq.preamp.toFixed(0)}</span>
          <div class="vert">
            <input type="range" min="-20" max="20" step="0.1" value={eq.preamp} aria-label="Preamp, {db(eq.preamp)}"
              style="--progress: {((eq.preamp + 20) / 40) * 100}%" oninput={(e) => setEq({ preamp: Number(e.currentTarget.value), preset: eq.preset })} />
          </div>
          <span class="hz">Pre</span>
        </div>
        {#each bands as hz, i}
          {@const v = eq.bands[i] ?? 0}
          <div class="band">
            <span class="tnum val">{v.toFixed(0)}</span>
            <div class="vert">
              <input type="range" min="-20" max="20" step="0.1" value={v} aria-label="{freq(hz)} Hz, {db(v)}" style="--progress: {((v + 20) / 40) * 100}%"
                oninput={(e) => setEq({ bands: eq.bands.map((b, j) => (j === i ? Number(e.currentTarget.value) : b)) })} />
            </div>
            <span class="hz">{freq(hz)}</span>
          </div>
        {/each}
      </div>
      <p class="note">dB, from −20 to +20. Stays on for everything you play until ReVLC quits.</p>
    {:else if tab === "colour"}
      <div class="line"><span>Adjust colours</span>{@render sw(fx.adjust.enabled, "Adjust colours", (on) => setAdjust({ enabled: on }))}</div>
      {#each ADJUST as [key, label, min, max, step, format]}
        {@render slider(label, toSlider(key, fx.adjust[key]), min, max, step, format(fx.adjust[key]),
          (v) => setAdjust({ [key]: fromSlider(key, v), enabled: true }), !fx.adjust.enabled)}
      {/each}
      <button class="btn reset" disabled={!adjustChanged} onclick={() => setAdjust({ ...DEFAULT_EFFECTS.adjust, enabled: fx.adjust.enabled })}>
        <RotateCcw size={14} />Reset
      </button>
    {:else}
      <h3>Logo</h3>
      <div class="line"><span>Show a logo</span>{@render sw(fx.logo.enabled, "Show a logo", (on) => (on && !fx.logo.file ? chooseLogo() : setLogo({ enabled: on })))}</div>
      <div class="line">
        <span class="file" data-tip={fx.logo.file || undefined}>{fx.logo.file ? fx.logo.file.split(/[\\/]/).pop() : "No image"}</span>
        <button class="btn" onclick={chooseLogo}>Choose…</button>
      </div>
      <div class="line"><span>Position</span><Select value={fx.logo.position} options={POSITIONS} onchange={(v) => setLogo({ position: Number(v) })} /></div>
      {@render slider("Opacity", fx.logo.opacity, 0, 255, 1, pct(fx.logo.opacity), (v) => setLogo({ opacity: v }))}

      <h3>Text</h3>
      <div class="line"><span>Show text</span>{@render sw(fx.marquee.enabled, "Show text", (on) => setMarquee({ enabled: on }))}</div>
      <input class="text" type="text" placeholder="Text, e.g. %H:%M or $t" value={fx.marquee.text} spellcheck="false" autocomplete="off" aria-label="Text"
        onkeydown={(e) => e.stopPropagation()} onchange={(e) => setMarquee({ text: e.currentTarget.value, enabled: e.currentTarget.value.trim() !== "" || fx.marquee.enabled })} />
      <p class="note">VLC's codes work: %H:%M for the time, $t for the title, $T for the position.</p>
      <div class="line"><span>Position</span><Select value={fx.marquee.position} options={POSITIONS} onchange={(v) => setMarquee({ position: Number(v) })} /></div>
      <div class="line"><span>Colour</span><Select value={fx.marquee.color} options={COLORS} onchange={(v) => setMarquee({ color: Number(v) })} /></div>
      <div class="line">
        <span>Size</span>
        <Select value={fx.marquee.size} options={[[0, "Automatic"], [16, "Small"], [24, "Medium"], [36, "Large"], [56, "Larger"]]} onchange={(v) => setMarquee({ size: Number(v) })} />
      </div>
      {@render slider("Opacity", fx.marquee.opacity, 0, 255, 1, pct(fx.marquee.opacity), (v) => setMarquee({ opacity: v }))}
    {/if}
  </div>
</aside>

<style>
  .effects {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 360px;
    display: flex;
    flex-direction: column;
    background: var(--panel);
    border-left: 1px solid var(--border);
    box-shadow: -12px 0 40px rgb(0 0 0 / 0.35);
    z-index: 21;
    font-size: 13.5px;
  }
  header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 44px 12px 6px 18px;
  }
  h2 {
    margin: 0;
    font-size: 17px;
    font-weight: 700;
  }
  h3 {
    margin: 18px 0 4px;
    font-size: 11px;
    font-weight: 600;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    color: var(--muted);
  }
  h3:first-child {
    margin-top: 0;
  }
  .icon {
    width: 32px;
    height: 32px;
    border-radius: 8px;
  }
  .icon:hover {
    background: var(--hover-strong);
  }
  .tabs {
    display: flex;
    gap: 2px;
    margin: 6px 16px 4px;
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
    flex: 1;
    overflow-y: auto;
    padding: 12px 18px 20px;
  }
  .line {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 12px;
    min-height: 38px;
  }
  .file {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--muted);
  }
  .muted {
    color: var(--muted);
  }
  .slider {
    padding: 6px 0;
  }
  .slider .top {
    display: flex;
    justify-content: space-between;
    margin-bottom: 4px;
  }
  .slider input {
    width: 100%;
  }
  .off {
    opacity: 0.45;
  }
  .eq {
    display: flex;
    justify-content: space-between;
    margin: 14px 0 6px;
  }
  .band {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
    width: 26px;
  }
  .band.pre {
    margin-right: 6px;
    padding-right: 6px;
    border-right: 1px solid var(--border);
    width: 30px;
  }
  /* A horizontal range turned upright, which every web view draws the same. */
  .vert {
    position: relative;
    width: 20px;
    height: 150px;
  }
  .vert input {
    position: absolute;
    top: 50%;
    left: 50%;
    width: 150px;
    transform: translate(-50%, -50%) rotate(-90deg);
  }
  .val,
  .hz {
    font-size: 10.5px;
    color: var(--muted);
  }
  .note {
    margin: 6px 0 0;
    font-size: 12px;
    color: var(--muted);
  }
  .reset {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    margin-top: 10px;
  }
  .text {
    box-sizing: border-box;
    width: 100%;
    margin-top: 4px;
    padding: 8px 10px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: rgb(0 0 0 / 0.3);
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .text:focus {
    border-color: var(--acc);
  }
  .switch {
    position: relative;
    flex-shrink: 0;
    width: 34px;
    height: 20px;
    border-radius: 10px;
    background: rgb(255 255 255 / 0.22);
    transition: background 0.15s;
  }
  .switch::after {
    content: "";
    position: absolute;
    left: 2px;
    top: 2px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: #fff;
    transition: transform 0.15s;
  }
  .switch.on {
    background: var(--acc);
  }
  .switch.on::after {
    transform: translateX(14px);
    background: var(--on-acc);
  }
</style>
