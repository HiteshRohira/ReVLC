<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { player, formatTime, parseTime } from "./player.svelte";
  import { ui } from "./ui.svelte";

  let text = $state(formatTime(player.state.time));
  let input: HTMLInputElement;

  const length = $derived(player.state.length);
  const ms = $derived(parseTime(text));
  const valid = $derived(ms !== null && (length === 0 || ms <= length));

  $effect(() => {
    input?.select();
  });

  function submit() {
    if (ms !== null) player.seek(ms);
    ui.dialog = null;
  }
</script>

<Dialog
  title="Jump to time"
  text={length > 0 ? `Hours, minutes and seconds, up to ${formatTime(length)}.` : "Hours, minutes and seconds."}
  submitLabel="Jump"
  canSubmit={valid && player.state.seekable}
  onsubmit={submit}
>
  <input bind:this={input} bind:value={text} type="text" class="tnum" placeholder="1:23:45" spellcheck="false" autocomplete="off" aria-invalid={!valid} />
</Dialog>
