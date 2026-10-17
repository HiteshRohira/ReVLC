<script lang="ts">
  import { Check } from "@lucide/svelte";
  import Transport from "./Transport.svelte";
  import { ACCENTS, theme, type AccentId } from "./theme.svelte";
</script>

<div class="onboarding" data-tauri-drag-region>
  <div class="card">
    <h1>Welcome to ReVLC</h1>
    <p class="lede">A modern player built on the VLC engine. Pick an accent to get started.</p>

    <div class="options" role="radiogroup" aria-label="Accent colour">
      {#each Object.entries(ACCENTS) as [id, a]}
        <button class="opt" class:on={theme.accent === id} role="radio" aria-checked={theme.accent === id}
          style="--c: {a.color}; --on: {a.on}" onclick={() => theme.setAccent(id as AccentId)}>
          <div class="preview">
            <div class="bar"><b></b><i></i></div>
            <div class="ctrls"><span class="play"><Transport playing={false} size={14} /></span><span class="pill">Open…</span></div>
          </div>
          <div class="label">
            <span class="dot"></span>{a.name}
            {#if theme.accent === id}<Check size={16} class="ck" />{/if}
          </div>
        </button>
      {/each}
    </div>

    <button class="go" onclick={() => theme.finishOnboarding()}>Get started</button>
    <p class="note">You can change this any time in Settings.</p>
    <p class="note">ReVLC sends anonymous statistics, like how quickly videos start, to help make playback better. Never what you watch. You can turn this off in Settings.</p>
  </div>
</div>

<style>
  .onboarding {
    position: absolute;
    inset: 0;
    display: grid;
    place-items: center;
    background: radial-gradient(ellipse at 50% 0%, #1c1c20, var(--bg) 65%);
  }
  .card {
    width: 560px;
    text-align: center;
  }
  h1 {
    margin: 0;
    font-size: 34px;
    font-weight: 800;
    letter-spacing: -0.02em;
  }
  .lede {
    margin: 10px 0 34px;
    color: var(--muted);
    font-size: 15px;
  }
  .options {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 16px;
  }
  .opt {
    display: flex;
    flex-direction: column;
    align-items: stretch;
    padding: 0;
    border-radius: 16px;
    background: var(--sidebar);
    box-shadow: 0 0 0 1px var(--border);
    overflow: hidden;
    text-align: left;
    transition: box-shadow 0.15s;
  }
  .opt.on {
    box-shadow: 0 0 0 2px var(--c);
  }
  .preview {
    height: 128px;
    padding: 20px 18px;
    display: flex;
    flex-direction: column;
    justify-content: flex-end;
    gap: 14px;
    background: linear-gradient(transparent, rgb(0 0 0 / 0.6)), linear-gradient(135deg, #2a3b4c, #3c2a3a 60%, #1b1b1f);
  }
  .bar {
    display: flex;
    gap: 3px;
    height: 4px;
  }
  .bar b {
    flex: 4;
    border-radius: 2px;
    background: var(--c);
  }
  .bar i {
    flex: 6;
    border-radius: 2px;
    background: rgb(255 255 255 / 0.25);
  }
  .ctrls {
    display: flex;
    align-items: center;
    justify-content: space-between;
  }
  .play {
    color: #fff;
  }
  .pill {
    padding: 4px 10px;
    border-radius: 7px;
    background: var(--c);
    color: var(--on);
    font-size: 11.5px;
    font-weight: 700;
  }
  .label {
    display: flex;
    justify-content: flex-start;
    align-items: center;
    gap: 10px;
    padding: 14px 16px;
    font-weight: 600;
  }
  .dot {
    width: 14px;
    height: 14px;
    border-radius: 50%;
    background: var(--c);
    box-shadow: 0 0 0 1px rgb(255 255 255 / 0.2);
  }
  .label :global(.ck) {
    margin-left: auto;
    color: var(--c);
  }
  .go {
    margin-top: 32px;
    padding: 12px 34px;
    border-radius: 12px;
    background: var(--acc);
    color: var(--on-acc);
    font-size: 15px;
    font-weight: 700;
  }
  .note {
    margin: 14px 0 0;
    font-size: 12px;
    color: var(--faint);
  }
  .note + .note {
    margin-top: 6px;
  }
</style>
