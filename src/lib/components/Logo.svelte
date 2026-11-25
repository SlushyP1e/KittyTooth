<script lang="ts">
  let { size = 22 }: { size?: number } = $props();
</script>

<svg viewBox="0 0 24 24" width={size} height={size} aria-hidden="true">
  <path
    d="M4.2 8.4 4.6 3.4l3.7 2.7A5.6 5.6 0 0 1 12 8.3a5.6 5.6 0 0 1 3.7-2.2l3.7-2.7.4 5c1.5 2.8-.1 5.8-2.4 8-1.8 1.8-5.4 4.1-5.4 4.1s-3.6-2.3-5.4-4.1c-2.3-2.2-3.9-5.2-2.4-8z"
  />
</svg>

<style>
  path {
    fill: none;
    stroke: var(--pink);
    stroke-width: 1.8;
    stroke-linejoin: round;
  }
</style>
