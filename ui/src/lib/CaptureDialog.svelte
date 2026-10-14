<script lang="ts">
  import { onMount } from "svelte";
  import { invoke } from "@tauri-apps/api/core";
  import Dialog from "./Dialog.svelte";
  import { player, type Capture, type Devices } from "./player.svelte";
  import { ui } from "./ui.svelte";

  type Kind = Capture["kind"];
  const KINDS: { kind: Kind; label: string }[] = [
    { kind: "camera", label: "Camera" },
    { kind: "microphone", label: "Microphone" },
    { kind: "screen", label: "Screen" },
  ];
  const FPS = [15, 24, 30, 60];

  let devices = $state<Devices>({ video: [], audio: [], screens: [] });
  let kind = $state<Kind>("camera");
  let device = $state("");
  let audio = $state("");
  let fps = $state(30);

  const list = $derived(kind === "camera" ? devices.video : kind === "microphone" ? devices.audio : devices.screens);

  function setKind(k: Kind) {
    kind = k;
    device = list[0]?.id ?? "";
  }

  function submit() {
    player.openCapture({ kind, device, audio: kind !== "microphone" && audio ? audio : null, fps: kind === "screen" ? fps : null });
    ui.dialog = null;
  }

  onMount(async () => {
    devices = await invoke<Devices>("capture_devices");
    device = list[0]?.id ?? "";
  });
</script>

<Dialog
  title="Open capture device"
  text="Play a camera, microphone or screen live. macOS asks for permission the first time."
  canSubmit={kind === "screen" || list.length > 0}
  onsubmit={submit}
>
  <div class="seg" role="radiogroup" aria-label="Device type">
    {#each KINDS as k}
      <button type="button" role="radio" aria-checked={kind === k.kind} class:on={kind === k.kind} onclick={() => setKind(k.kind)}>{k.label}</button>
    {/each}
  </div>

  <label for="capture-device">{kind === "screen" ? "Display" : "Device"}</label>
  <select id="capture-device" bind:value={device} disabled={!list.length}>
    {#each list as d (d.id)}<option value={d.id}>{d.name}</option>{:else}<option value="">No {kind} found</option>{/each}
  </select>

  {#if kind !== "microphone"}
    <label for="capture-audio">Audio</label>
    <select id="capture-audio" bind:value={audio}>
      <option value="">None</option>
      {#each devices.audio as d (d.id)}<option value={d.id}>{d.name}</option>{/each}
    </select>
  {/if}

  {#if kind === "screen"}
    <label for="capture-fps">Frame rate</label>
    <select id="capture-fps" bind:value={fps}>
      {#each FPS as f}<option value={f}>{f} fps</option>{/each}
    </select>
  {/if}
</Dialog>
