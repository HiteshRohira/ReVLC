<script lang="ts">
  import { onMount } from "svelte";
  import { getVersion } from "@tauri-apps/api/app";
  import { invoke } from "@tauri-apps/api/core";
  import { open as openDialog } from "@tauri-apps/plugin-dialog";
  import { Captions, Cpu, Keyboard, MonitorPlay, RotateCcw, SlidersHorizontal, Volume2 } from "@lucide/svelte";
  import { ACTIONS, FIXED_SHORTCUTS, effectiveBindings, eventBinding, formatBinding } from "./actions";
  import { settings, type Settings } from "./settings.svelte";
  import { ACCENTS, theme, type AccentId } from "./theme.svelte";
  import Row from "./Row.svelte";
  import Select from "./Select.svelte";
  import WindowControls from "./WindowControls.svelte";
  import { isMac } from "./platform";

  // VLC's default snapshot folder.
  const SNAPSHOT_DEFAULT = isMac ? "Desktop" : "Pictures";

  const TABS = [
    { id: "general", label: "General", icon: SlidersHorizontal },
    { id: "video", label: "Video", icon: MonitorPlay },
    { id: "audio", label: "Audio", icon: Volume2 },
    { id: "subtitles", label: "Subtitles", icon: Captions },
    { id: "input", label: "Input & Codecs", icon: Cpu },
    { id: "shortcuts", label: "Shortcuts", icon: Keyboard },
  ] as const;
  type Tab = (typeof TABS)[number]["id"];
  let tab = $state<Tab>("general");
  let version = $state("");
  getVersion().then((v) => (version = v));
  // Results show in the main window.
  const checkForUpdates = () => invoke("update_check");
  let mainEl = $state<HTMLElement>();
  // Each tab starts at the top.
  $effect(() => {
    tab;
    mainEl?.scrollTo(0, 0);
  });

  // Keys reset by each tab's "Restore defaults".
  const TAB_KEYS: Record<Exclude<Tab, "shortcuts">, (keyof Settings)[]> = {
    general: ["resume", "stopAfterEach", "pauseWhenMinimized", "seekbarChapters", "autoUpdate", "analytics"],
    video: ["video", "startFullscreen", "alwaysOnTop", "deinterlace", "deinterlaceMode", "snapshotPath", "snapshotPrefix", "snapshotFormat", "snapshotSequential"],
    audio: ["audio", "audioDevice", "audioLanguage", "timeStretch", "replayGain", "spdif"],
    subtitles: ["subtitleLanguage", "subAutodetect", "subAutodetectFuzzy", "subEncoding", "font", "fontSize", "fontColor", "fontOpacity", "bold", "outlineThickness", "outlineColor", "backgroundOpacity", "backgroundColor", "shadowOpacity", "subMargin", "showTitle", "titleTimeout", "titlePosition"],
    input: ["hardwareDecoding", "fileCaching", "networkCaching", "skipLoopFilter"],
  };

  // VLC's colour choices for subtitle text, outline and background.
  const COLORS: [number, string][] = [
    [0xffffff, "White"], [0x000000, "Black"], [0x808080, "Gray"], [0xc0c0c0, "Silver"], [0xffff00, "Yellow"],
    [0x800000, "Maroon"], [0xff0000, "Red"], [0xff00ff, "Fuchsia"], [0x808000, "Olive"], [0x008000, "Green"],
    [0x008080, "Teal"], [0x00ff00, "Lime"], [0x800080, "Purple"], [0x000080, "Navy"], [0x0000ff, "Blue"], [0x00ffff, "Aqua"],
  ];
  const FONT_SIZES: [number, string][] = [[0, "Auto"], [20, "Smaller"], [18, "Small"], [16, "Normal"], [12, "Large"], [6, "Larger"]];
  const OUTLINES: [number, string][] = [[0, "None"], [2, "Thin"], [4, "Normal"], [6, "Thick"]];
  const FUZZY: [number, string][] = [
    [0, "Don't load subtitle files"], [1, "Any subtitle file in the folder"], [2, "Files containing the video's name"],
    [3, "Files starting with the video's name"], [4, "Files with exactly the video's name"],
  ];
  const ENCODINGS = ["", "UTF-8", "UTF-16", "Windows-1252", "ISO-8859-15", "IBM850", "ISO-8859-2", "Windows-1250", "Windows-1251", "KOI8-R", "KOI8-U", "Windows-1256", "ISO-8859-7", "Windows-1253", "Windows-1255", "Windows-1254", "Windows-874", "Windows-1257", "GB18030", "Big5", "Shift_JIS", "EUC-JP", "CP949", "EUC-KR", "VISCII", "Windows-1258"];
  const TITLE_POSITIONS: [number, string][] = [[8, "Bottom"], [4, "Top"], [0, "Center"], [9, "Bottom left"], [10, "Bottom right"], [5, "Top left"], [6, "Top right"]];
  const DEINTERLACE_MODES = ["auto", "discard", "blend", "mean", "bob", "linear", "x", "yadif", "yadif2x", "phosphor", "ivtc"];
  const LOOP_FILTER: [number, string][] = [[0, "None"], [1, "Non-reference frames"], [2, "Bidirectional frames"], [3, "Non-key frames"], [4, "All frames"]];

  const v = $derived(settings.value);
  const info = $derived(settings.info);
  // Whether this tab differs from the defaults, so "Restore defaults" has something to do.
  const tabChanged = $derived.by(() => {
    if (!v || !info) return false;
    if (tab === "shortcuts") return Object.keys(v.shortcuts).length > 0;
    return TAB_KEYS[tab].some((k) => JSON.stringify(v[k]) !== JSON.stringify(info.defaults[k]));
  });
  const tabLabel = $derived(TABS.find((t) => t.id === tab)!.label);
  const pct = (n: number) => `${Math.round((n / 255) * 100)}%`;
  const hex = (n: number) => `#${n.toString(16).padStart(6, "0")}`;

  // Shortcut recording.
  let recording = $state<string | null>(null);
  let conflict = $state<string | null>(null);
  const bindings = $derived(effectiveBindings(v?.shortcuts ?? {}));

  function setBinding(id: string, binding: string) {
    const overrides = { ...(v?.shortcuts ?? {}) };
    conflict = null;
    // A binding can only do one thing: take it from any other action.
    for (const a of ACTIONS) {
      if (a.id !== id && binding && bindings[a.id] === binding) {
        overrides[a.id] = "";
        conflict = `${formatBinding(binding)} was used by “${a.label}”, which now has no shortcut.`;
      }
    }
    const action = ACTIONS.find((a) => a.id === id)!;
    if (binding === action.keys) delete overrides[id];
    else overrides[id] = binding;
    for (const a of ACTIONS) if (overrides[a.id] === a.keys) delete overrides[a.id];
    settings.set({ shortcuts: overrides });
  }

  function onKey(e: KeyboardEvent) {
    if (!recording) return;
    e.preventDefault();
    e.stopPropagation();
    if (e.key === "Escape") return (recording = null);
    if (e.key === "Backspace" || e.key === "Delete") {
      setBinding(recording, "");
      return (recording = null);
    }
    const b = eventBinding(e);
    if (!b) return;
    setBinding(recording, b);
    recording = null;
  }

  async function chooseSnapshotFolder() {
    const dir = await openDialog({ directory: true, title: "Snapshot folder" });
    if (typeof dir === "string") settings.set({ snapshotPath: dir });
  }

  onMount(() => {
    settings.load();
  });
</script>

<svelte:window onkeydowncapture={onKey} />

{#snippet toggle(key: keyof Settings)}
  <!-- svelte-ignore a11y_consider_explicit_label (Row names it after its label) -->
  <button class="switch" class:on={v![key]} role="switch" aria-checked={!!v![key]}
    onclick={() => settings.set({ [key]: !v![key] } as Partial<Settings>)}></button>
{/snippet}

{#snippet choice(key: keyof Settings, options: [number | string, string][], debounce = false)}
  <Select value={v![key] as number | string} {options} onchange={(val) => settings.set({ [key]: val } as Partial<Settings>, debounce)} />
{/snippet}

{#snippet color(key: keyof Settings)}
  <span class="swatch" style="background: {hex(v![key] as number)}"></span>
  {@render choice(key, COLORS)}
{/snippet}

{#snippet number(key: keyof Settings, unit: string, min: number, max: number, step = 100)}
  <input class="num" type="number" {min} {max} {step} value={v![key] as number}
    onchange={(e) => settings.set({ [key]: Math.max(min, Math.min(max, Number(e.currentTarget.value) || 0)) } as Partial<Settings>)} />
  <span class="unit">{unit}</span>
{/snippet}

{#snippet slider(key: keyof Settings, max: number, format: (n: number) => string)}
  <input type="range" min="0" {max} value={v![key] as number} style="--progress: {((v![key] as number) / max) * 100}%"
    oninput={(e) => settings.set({ [key]: Number(e.currentTarget.value) } as Partial<Settings>, true)} />
  <span class="unit tnum">{format(v![key] as number)}</span>
{/snippet}

{#snippet text(key: keyof Settings, placeholder: string)}
  <input class="text" type="text" {placeholder} value={(v![key] as string) ?? ""} spellcheck="false"
    onchange={(e) => settings.set({ [key]: e.currentTarget.value.trim() } as Partial<Settings>)} />
{/snippet}

<div class="settings">
  {#if !isMac}<WindowControls />{/if}
  <header data-tauri-drag-region>
    <nav data-tauri-drag-region>
      {#each TABS as t}
        <button class="tab" class:on={tab === t.id} onclick={() => { tab = t.id; recording = null; conflict = null; }}>
          <t.icon size={22} strokeWidth={1.6} /><span>{t.label}</span>
        </button>
      {/each}
    </nav>
  </header>

  <main bind:this={mainEl}>
    {#if settings.error}<p class="save-error" role="alert">{settings.error}</p>{/if}
    {#if !v || !info}
      <p class="muted">Loading…</p>
    {:else if tab === "general"}
      <h3>Appearance</h3>
      <div class="group">
        <Row label="Accent colour">
          <div class="accents">
            {#each Object.entries(ACCENTS) as [id, a]}
              <button class="accent" class:on={theme.accent === id} style="--c: {a.color}" onclick={() => theme.setAccent(id as AccentId)}>
                <span></span>{a.name}
              </button>
            {/each}
          </div>
        </Row>
        <Row label="Show chapters on the seek bar" help="Split the seek bar where the file has chapter markers.">{@render toggle("seekbarChapters")}</Row>
      </div>
      <h3>Playback</h3>
      <div class="group">
        <Row label="Continue where you left off" help="When reopening a video you stopped part-way through.">
          {@render choice("resume", [["ask", "Ask"], ["always", "Always"], ["never", "Never"]])}
        </Row>
        <Row label="When a video ends">
          <Select value={v.stopAfterEach ? "stop" : "next"} options={[["next", "Play the next item"], ["stop", "Stop"]]}
            onchange={(val) => settings.set({ stopAfterEach: val === "stop" })} />
        </Row>
        <Row label="Pause when minimised">{@render toggle("pauseWhenMinimized")}</Row>
      </div>
      <h3>Updates</h3>
      <div class="group">
        <Row label="Install updates automatically" help="New versions download in the background; you choose when to restart.">
          {@render toggle("autoUpdate")}
        </Row>
        <Row label="ReVLC {version}">
          <button class="btn" onclick={checkForUpdates}>Check now</button>
        </Row>
      </div>
      <h3>Privacy</h3>
      <div class="group">
        <Row label="Share usage statistics"
          help="Your app version and computer type, and how quickly videos start and seek. No personal information, never what you watch.">
          {@render toggle("analytics")}
        </Row>
      </div>
    {:else if tab === "video"}
      <h3>Display</h3>
      <div class="group">
        <Row label="Enable video" help="Turn off to play only the audio of videos.">{@render toggle("video")}</Row>
        <Row label="Start in fullscreen">{@render toggle("startFullscreen")}</Row>
        <Row label="Keep window on top">{@render toggle("alwaysOnTop")}</Row>
      </div>
      <h3>Deinterlacing</h3>
      <div class="group">
        <Row label="Deinterlacing">{@render choice("deinterlace", [[-1, "Automatic"], [1, "On"], [0, "Off"]])}</Row>
        <Row label="Method">
          {@render choice("deinterlaceMode", DEINTERLACE_MODES.map((m) => [m, m === "auto" ? "Automatic" : m === "yadif2x" ? "Yadif (2x)" : m[0].toUpperCase() + m.slice(1)]))}
        </Row>
      </div>
      <h3>Snapshots</h3>
      <div class="group">
        <Row label="Folder">
          <span class="path" data-tip={v.snapshotPath || SNAPSHOT_DEFAULT}>{v.snapshotPath ? v.snapshotPath.split(/[\\/]/).pop() : SNAPSHOT_DEFAULT}</span>
          <button class="btn" onclick={chooseSnapshotFolder}>Choose…</button>
        </Row>
        <Row label="File name prefix">{@render text("snapshotPrefix", "vlcsnap-")}</Row>
        <Row label="Format">{@render choice("snapshotFormat", [["png", "PNG"], ["jpg", "JPEG"], ["tiff", "TIFF"]])}</Row>
        <Row label="Number files sequentially" help="Instead of using timestamps.">{@render toggle("snapshotSequential")}</Row>
      </div>
    {:else if tab === "audio"}
      <h3>Output</h3>
      <div class="group">
        <Row label="Enable audio">{@render toggle("audio")}</Row>
        <Row label="Output device">
          {@render choice("audioDevice", info.audioDevices.length ? info.audioDevices.map(([id, name]) => [id, id === "" ? "System default" : name]) : [["", "System default"]])}
        </Row>
        <Row label="Force S/PDIF passthrough" help="For receivers that can't negotiate it automatically.">{@render toggle("spdif")}</Row>
      </div>
      <h3>Tracks</h3>
      <div class="group">
        <Row label="Preferred audio language" help="Two- or three-letter codes, comma separated (e.g. en, ja).">{@render text("audioLanguage", "Any")}</Row>
        <Row label="Keep pitch when changing speed">{@render toggle("timeStretch")}</Row>
        <Row label="Replay gain">{@render choice("replayGain", [["none", "Off"], ["track", "Track"], ["album", "Album"]])}</Row>
      </div>
    {:else if tab === "subtitles"}
      <div class="preview" aria-hidden="true">
        <span
          style="font-family: '{v.font}'; font-weight: {v.bold ? 700 : 500};
            font-size: {({ 0: 26, 20: 18, 18: 22, 16: 26, 12: 32, 6: 40 } as Record<number, number>)[v.fontSize] ?? 26}px;
            color: {hex(v.fontColor)}; opacity: {v.fontOpacity / 255};
            -webkit-text-stroke: {v.outlineThickness / 2}px {hex(v.outlineColor)}; paint-order: stroke fill;
            text-shadow: 2px 2px 3px rgb(0 0 0 / {v.shadowOpacity / 255});
            background: {v.backgroundOpacity ? `${hex(v.backgroundColor)}${Math.round(v.backgroundOpacity).toString(16).padStart(2, '0')}` : 'transparent'};
            margin-bottom: {Math.min(60, v.subMargin / 4)}px"
        >This is how subtitles will look.</span>
        <small class="credit">Tears of Steel · (CC) Blender Foundation</small>
      </div>
      <h3>Language</h3>
      <div class="group">
        <Row label="Preferred language" help="Subtitles in these languages turn on automatically. VLC's default is none.">
          <Select value={v.subtitleLanguage === null ? "system" : "custom"}
            options={[["system", `System languages (${info.systemLanguages.join(", ") || "none"})`], ["custom", "Custom…"]]}
            onchange={(val) => settings.set({ subtitleLanguage: val === "system" ? null : info.systemLanguages.join(",") })} />
        </Row>
        {#if v.subtitleLanguage !== null}
          <Row label="Languages" help="Two- or three-letter codes; add “any” to fall back to any track.">{@render text("subtitleLanguage", "en, fr")}</Row>
        {/if}
        <Row label="Load matching subtitle files">{@render toggle("subAutodetect")}</Row>
        {#if v.subAutodetect}<Row label="Match">{@render choice("subAutodetectFuzzy", FUZZY)}</Row>{/if}
        <Row label="Text encoding" help="For text subtitle files that show garbled characters.">{@render choice("subEncoding", ENCODINGS.map((e) => [e, e || "Automatic"]))}</Row>
      </div>
      <h3>Appearance</h3>
      <div class="group">
        <Row label="Font">{@render choice("font", (info.fonts.length ? info.fonts : [v.font]).map((f) => [f, f]))}</Row>
        <Row label="Size">{@render choice("fontSize", FONT_SIZES)}</Row>
        <Row label="Colour">{@render color("fontColor")}</Row>
        <Row label="Opacity">{@render slider("fontOpacity", 255, pct)}</Row>
        <Row label="Bold">{@render toggle("bold")}</Row>
        <Row label="Outline">{@render choice("outlineThickness", OUTLINES)}</Row>
        {#if v.outlineThickness > 0}<Row label="Outline colour">{@render color("outlineColor")}</Row>{/if}
        <Row label="Shadow">{@render slider("shadowOpacity", 255, pct)}</Row>
        <Row label="Background">{@render slider("backgroundOpacity", 255, pct)}</Row>
        {#if v.backgroundOpacity > 0}<Row label="Background colour">{@render color("backgroundColor")}</Row>{/if}
        <Row label="Raise subtitles by" help="Moves subtitles up from the bottom of the video.">{@render number("subMargin", "px", 0, 1000, 10)}</Row>
      </div>
      <h3>On-screen title</h3>
      <div class="group">
        <Row label="Show the title when a video starts">{@render toggle("showTitle")}</Row>
        {#if v.showTitle}
          <Row label="For">{@render number("titleTimeout", "ms", 500, 60000, 500)}</Row>
          <Row label="Position">{@render choice("titlePosition", TITLE_POSITIONS)}</Row>
        {/if}
      </div>
    {:else if tab === "input"}
      <h3>Decoding</h3>
      <div class="group">
        <Row label="Hardware-accelerated decoding" help={isMac ? "Uses the Mac's video decoder (VideoToolbox). Saves battery." : "Uses the graphics card's video decoder (Direct3D 11). Saves battery."}>{@render toggle("hardwareDecoding")}</Row>
        <Row label="Skip H.264 loop filter" help="Speeds up software decoding of H.264 at some cost in quality.">{@render choice("skipLoopFilter", LOOP_FILTER)}</Row>
      </div>
      <h3>Caching</h3>
      <div class="group">
        <Row label="File caching">{@render number("fileCaching", "ms", 0, 60000)}</Row>
        <Row label="Network caching" help="Raise this if streams stutter.">{@render number("networkCaching", "ms", 0, 60000)}</Row>
      </div>
    {:else if tab === "shortcuts"}
      <p class="intro">Click a shortcut and press new keys. <kbd>Delete</kbd> removes it, <kbd>Esc</kbd> cancels.</p>
      {#if conflict}<p class="conflict">{conflict}</p>{/if}
      {#each ["Playback", "Audio", "Video", "Subtitles", "App"] as group}
        <h3>{group}</h3>
        <div class="group">
          {#each ACTIONS.filter((a) => a.group === group) as a (a.id)}
            <Row label={a.label}>
              <button class="key" class:rec={recording === a.id} class:changed={bindings[a.id] !== a.keys}
                onclick={() => { recording = recording === a.id ? null : a.id; conflict = null; }}>
                {recording === a.id ? "Press keys…" : formatBinding(bindings[a.id]) || "None"}
              </button>
            </Row>
          {/each}
        </div>
      {/each}
      <h3>Fixed</h3>
      <div class="group">
        {#each FIXED_SHORTCUTS as [k, label]}
          <Row {label}><span class="key fixed">{formatBinding(k)}</span></Row>
        {/each}
      </div>
    {/if}

    {#if tabChanged}
      <footer>
        <button class="btn" onclick={() => (tab === "shortcuts" ? settings.set({ shortcuts: {} }) : settings.reset(TAB_KEYS[tab]))}>
          <RotateCcw size={14} /> Restore {tabLabel} defaults
        </button>
      </footer>
    {/if}
  </main>
</div>

<style>
  .settings {
    position: fixed;
    inset: 0;
    display: flex;
    flex-direction: column;
    background: var(--bg);
  }
  header {
    flex-shrink: 0;
    padding: 30px 16px 8px;
    border-bottom: 1px solid var(--border);
    background: var(--sidebar);
  }
  nav {
    display: flex;
    justify-content: center;
    gap: 4px;
  }
  .tab {
    display: flex;
    flex-direction: column;
    gap: 4px;
    min-width: 82px;
    padding: 7px 10px;
    border-radius: 9px;
    color: var(--muted);
    font-size: 11.5px;
    font-weight: 500;
  }
  .tab:hover {
    color: var(--text);
  }
  .tab.on {
    color: var(--acc);
    background: var(--acc-soft);
  }
  main {
    flex: 1;
    overflow-y: auto;
    padding: 6px 28px 24px;
  }
  h3 {
    margin: 20px 4px 8px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  .group :global(.row + .row) {
    border-top: 1px solid var(--border);
  }
  .group {
    border-radius: 12px;
    background: var(--sidebar);
    border: 1px solid var(--border);
  }
  .text,
  .num {
    height: 28px;
    padding: 0 10px;
    border-radius: 7px;
    border: 1px solid var(--border);
    background: rgb(255 255 255 / 0.06);
    color: var(--text);
    font: inherit;
    font-size: 13px;
    max-width: 280px;
  }
  .text:focus,
  .num:focus {
    outline: 2px solid var(--acc);
    outline-offset: 0;
  }
  .text {
    width: 200px;
  }
  .num {
    width: 90px;
    text-align: right;
  }
  /* The webview's spin arrows; the arrow keys still step the value. */
  .num::-webkit-inner-spin-button,
  .num::-webkit-outer-spin-button {
    -webkit-appearance: none;
    margin: 0;
  }
  .unit {
    min-width: 34px;
    font-size: 12px;
    color: var(--muted);
  }
  input[type="range"] {
    width: 160px;
  }
  .swatch {
    width: 16px;
    height: 16px;
    border-radius: 50%;
    box-shadow: 0 0 0 1px rgb(255 255 255 / 0.25);
  }
  .switch {
    position: relative;
    width: 38px;
    height: 22px;
    border-radius: 11px;
    background: rgb(255 255 255 / 0.18);
    transition: background 0.15s;
  }
  .switch::after {
    content: "";
    position: absolute;
    left: 2px;
    top: 2px;
    width: 18px;
    height: 18px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 2px rgb(0 0 0 / 0.4);
    transition: transform 0.15s;
  }
  .switch.on {
    background: var(--acc);
  }
  .switch.on::after {
    transform: translateX(16px);
    background: var(--on-acc);
  }
  .accents {
    display: flex;
    gap: 8px;
  }
  .accent {
    display: inline-flex;
    gap: 8px;
    padding: 5px 12px 5px 8px;
    border-radius: 8px;
    font-size: 13px;
    box-shadow: 0 0 0 1px var(--border);
  }
  .accent span {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--c);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 0.2);
  }
  .accent.on {
    box-shadow: 0 0 0 2px var(--c);
  }
  .path {
    max-width: 180px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: 13px;
    color: var(--muted);
  }
  .btn {
    height: 28px;
    padding: 0 12px;
    font-size: 12.5px;
  }
  /* Stays in view while the appearance controls below are adjusted. */
  .preview {
    position: sticky;
    top: 0;
    z-index: 2;
    display: flex;
    align-items: flex-end;
    justify-content: center;
    height: 120px;
    margin-top: 16px;
    box-shadow: 0 8px 24px var(--bg);
    padding: 0 20px 22px;
    border-radius: 12px;
    background: linear-gradient(transparent 40%, rgb(0 0 0 / 0.35)), url("/preview-frame.jpg") center 35% / cover, #2a3340;
    overflow: hidden;
  }
  .preview .credit {
    position: absolute;
    top: 8px;
    right: 10px;
    padding: 1px 6px;
    border-radius: 4px;
    background: rgb(0 0 0 / 0.5);
    font-size: 10px;
    color: rgb(255 255 255 / 0.75);
  }
  .preview span {
    padding: 2px 6px;
    text-align: center;
    line-height: 1.2;
  }
  .intro {
    margin: 18px 4px 0;
    font-size: 13px;
    color: var(--muted);
  }
  .save-error {
    margin: 0 4px 14px;
    font-size: 12.5px;
    color: #ff7b72;
  }
  .conflict {
    margin: 10px 4px 0;
    font-size: 12.5px;
    color: #ffb36b;
  }
  .key {
    min-width: 92px;
    height: 28px;
    padding: 0 10px;
    border-radius: 7px;
    background: rgb(255 255 255 / 0.08);
    font-size: 13px;
    font-variant-numeric: tabular-nums;
  }
  .key.changed {
    box-shadow: inset 0 0 0 1px var(--acc);
  }
  .key.rec {
    background: var(--acc);
    color: var(--on-acc);
  }
  .key.fixed {
    display: inline-grid;
    place-items: center;
    color: var(--muted);
  }
  footer {
    display: flex;
    justify-content: flex-end;
    margin-top: 22px;
  }
  .muted {
    color: var(--muted);
  }
</style>
