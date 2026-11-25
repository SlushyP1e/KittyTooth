<script lang="ts">
  import { rssiBars, rssiLabel } from "../format";
  let { rssi }: { rssi: number | null } = $props();
  const bars = $derived(rssiBars(rssi));
</script>

<span class="rssi" title={rssi == null ? "Signal unknown" : `${rssi} dBm`}>
  <svg viewBox="0 0 20 16" width="20" height="16" aria-hidden="true">
    {#each [0, 1, 2, 3] as i (i)}
      <rect x={i * 5} y={12 - i * 4} width="3.4" height={4 + i * 4} rx="1" class:on={i < bars} />
    {/each}
  </svg>
  <span class="sr-only">Signal {rssiLabel(rssi)}</span>
</span>

<style>
  .rssi {
    display: inline-flex;
    align-items: center;
  }
  rect {
    fill: var(--surface-3);
  }
  rect.on {
    fill: var(--accent);
  }
</style>
