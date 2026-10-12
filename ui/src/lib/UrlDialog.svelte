<script lang="ts">
  import { player } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let url = $state("");
  let input: HTMLInputElement;

  $effect(() => {
    input?.focus();
  });

  function submit(e: SubmitEvent) {
    e.preventDefault();
    const value = url.trim();
    if (!value) return;
    player.open([value.includes("://") ? value : `https://${value}`]);
    ui.urlOpen = false;
  }
</script>

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && (ui.urlOpen = false)}>
  <div class="dialog" role="dialog" aria-label="Open URL"><form onsubmit={submit}>
    <h2>Open network stream</h2>
    <p>HTTP(S), HLS, DASH, RTSP, RTMP, UDP, or a YouTube link.</p>
    <input
      bind:this={input}
      bind:value={url}
      type="text"
      placeholder="https://example.com/video.m3u8"
      spellcheck="false"
      autocomplete="off"
      onkeydown={(e) => e.key === "Escape" && (ui.urlOpen = false)}
    />
    <div class="buttons">
      <button type="button" class="btn" onclick={() => (ui.urlOpen = false)}>Cancel</button>
      <button type="submit" class="btn primary" disabled={!url.trim()}>Play</button>
    </div>
  </form></div>
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
    width: min(520px, calc(100vw - 48px));
    padding: 20px;
    border-radius: 14px;
    background: var(--panel);
    border: 1px solid var(--border);
    box-shadow: 0 20px 60px rgb(0 0 0 / 0.6);
  }
  h2 {
    margin: 0 0 4px;
    font-size: 16px;
  }
  p {
    margin: 0 0 14px;
    color: var(--muted);
    font-size: 13px;
  }
  input {
    width: 100%;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: rgb(0 0 0 / 0.3);
    color: var(--text);
    font: inherit;
    outline: none;
  }
  input:focus {
    border-color: var(--accent);
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 16px;
  }
</style>
