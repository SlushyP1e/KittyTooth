<script lang="ts">
  let { size = 28 }: { size?: number } = $props();
</script>

<svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true" class="logo">
  <path
    d="M3.4 8.1 3.9 2.6l4.3 3.1A6.1 6.1 0 0 1 12 8c1.1-1.2 2.3-2 3.8-2.3l4.3-3.1.5 5.5c1.6 3-.1 6.2-2.6 8.6C16 18.6 12 21.2 12 21.2s-4-2.6-6-4.5C3.5 14.3 1.8 11.1 3.4 8.1z"
  />
  <path class="ecg" d="M5.6 12.6h2.6l1.3-2.6 2 4.6 1.5-3.2.9 1.2h4.5" />
</svg>

<style>
  .logo path:first-child {
    fill: var(--accent);
  }
  .ecg {
    fill: none;
    stroke: var(--accent-text);
    stroke-width: 1.3;
    stroke-linecap: round;
    stroke-linejoin: round;
    opacity: 0.85;
  }
</style>
