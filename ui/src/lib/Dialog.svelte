<script lang="ts">
  import type { Snippet } from "svelte";
  import { ui } from "./ui.svelte";

  let {
    title,
    text,
    submitLabel = "Play",
    canSubmit = true,
    onsubmit,
    children,
  }: { title: string; text?: string; submitLabel?: string; canSubmit?: boolean; onsubmit: () => void; children: Snippet } = $props();

  const close = () => (ui.dialog = null);

  function submit(e: SubmitEvent) {
    e.preventDefault();
    if (canSubmit) onsubmit();
  }
</script>

<svelte:window onkeydown={(e) => e.key === "Escape" && close()} />

<div class="backdrop" role="presentation" onpointerdown={(e) => e.target === e.currentTarget && close()}>
  <div class="dialog" role="dialog" aria-label={title}><form onsubmit={submit}>
    <h2>{title}</h2>
    {#if text}<p>{text}</p>{/if}
    {@render children()}
    <div class="buttons">
      <button type="button" class="btn" onclick={close}>Cancel</button>
      <button type="submit" class="btn primary" disabled={!canSubmit}>{submitLabel}</button>
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
  .dialog :global(input[type="text"]),
  .dialog :global(.select.field) {
    box-sizing: border-box;
    width: 100%;
    max-width: none;
    height: auto;
    padding: 10px 12px;
    border-radius: 8px;
    border: 1px solid var(--border);
    background: rgb(0 0 0 / 0.3);
    color: var(--text);
    font: inherit;
    outline: none;
  }
  .dialog :global(input[type="text"]:focus),
  .dialog :global(.select.field:focus-visible),
  .dialog :global(.select.field.open) {
    border-color: var(--acc);
  }
  .dialog :global(label) {
    display: block;
    margin: 14px 0 6px;
    font-size: 12px;
    font-weight: 600;
    color: var(--muted);
  }
  /* Drawn here rather than by the webview, which uses the Windows control. */
  .dialog :global(input[type="checkbox"]) {
    -webkit-appearance: none;
    appearance: none;
    display: inline-grid;
    place-items: center;
    width: 16px;
    height: 16px;
    margin: 0 6px 0 0;
    vertical-align: -3px;
    border-radius: 4px;
    border: 1px solid rgb(255 255 255 / 0.3);
    background: rgb(0 0 0 / 0.3);
  }
  .dialog :global(input[type="checkbox"]:checked) {
    border-color: var(--acc);
    background: var(--acc);
  }
  .dialog :global(input[type="checkbox"]:checked::after) {
    content: "";
    width: 4px;
    height: 8px;
    margin-top: -2px;
    border: solid var(--on-acc);
    border-width: 0 2px 2px 0;
    transform: rotate(45deg);
  }
  /* Segmented control: a row of mutually exclusive buttons. */
  .dialog :global(.seg) {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: 9px;
    background: rgb(0 0 0 / 0.3);
  }
  .dialog :global(.seg button) {
    flex: 1;
    height: 30px;
    border-radius: 7px;
    font-size: 13px;
    color: var(--muted);
  }
  .dialog :global(.seg button.on) {
    background: var(--hover-strong);
    color: var(--text);
    font-weight: 600;
  }
  .buttons {
    display: flex;
    justify-content: flex-end;
    gap: 8px;
    margin-top: 18px;
  }
</style>
