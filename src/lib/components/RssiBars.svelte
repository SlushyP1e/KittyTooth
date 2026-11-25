<script lang="ts">
  import { rssiBars, rssiLabel } from "../format";
  let { rssi }: { rssi: number | null } = $props();
  const bars = $derived(rssiBars(rssi));
</script>

<span class="rssi" title={rssi == null ? "Signal unknown" : `${rssi} dBm`}>
  {#each [0, 1, 2, 3] as i (i)}
    <span class="bar" class:on={i < bars} style:height="{5 + i * 3}px"></span>
  {/each}
  <span class="sr-only">Signal {rssiLabel(rssi)}</span>
</span>

<style>
  .rssi {
    display: inline-flex;
    align-items: flex-end;
    gap: 2px;
    height: 14px;
  }
  .bar {
    width: 3px;
    border-radius: 1px;
    background: var(--slate-3);
  }
  .bar.on {
    background: var(--pink);
  }
</style>
