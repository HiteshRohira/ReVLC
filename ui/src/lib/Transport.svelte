<script lang="ts">
  // Filled play/pause glyphs; outlines read too light for the main control.
  let { playing, size = 24 }: { playing: boolean; size?: number } = $props();
</script>

<svg width={size} height={size} viewBox="0 0 24 24" fill="currentColor" aria-hidden="true">
  {#if playing}
    <rect x="5.5" y="4" width="4.5" height="16" rx="1.2" />
    <rect x="14" y="4" width="4.5" height="16" rx="1.2" />
  {:else}
    <path d="M7 4.6v14.8c0 .8.9 1.3 1.6.9l11.6-7.4c.6-.4.6-1.3 0-1.7L8.6 3.7C7.9 3.3 7 3.8 7 4.6z" />
  {/if}
</svg>
