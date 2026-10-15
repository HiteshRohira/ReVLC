<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { player } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let url = $state("");
  let input: HTMLInputElement;

  $effect(() => {
    input?.focus();
  });

  function submit() {
    const value = url.trim();
    player.open([value.includes("://") ? value : `https://${value}`]);
    ui.dialog = null;
  }
</script>

<Dialog title="Open network stream" text="HTTP(S), HLS, DASH, RTSP, RTMP or UDP." canSubmit={!!url.trim()} onsubmit={submit}>
  <input bind:this={input} bind:value={url} type="text" placeholder="https://example.com/video.m3u8" spellcheck="false" autocomplete="off" />
</Dialog>
