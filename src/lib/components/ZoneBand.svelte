<script lang="ts">
  import { zoneName } from "../format";

  // One thin track: current zone in full accent, lower zones at 35 %, higher zones empty.
  // Zones are told apart by their labels, not by hue.
  let {
    bpm,
    maxHr,
    thresholds,
    zone,
    active,
  }: { bpm: number | null; maxHr: number; thresholds: number[]; zone: number; active: boolean } = $props();

  const LO = 40;
  const pos = (pct: number) => Math.max(0, Math.min(100, ((pct - LO) / (100 - LO)) * 100));
  const edges = $derived([LO, ...thresholds, 100]);
  const pct = $derived(bpm == null ? null : (bpm * 100) / maxHr);
</script>

<div class="zone">
  <div class="track" role="meter" aria-label="Heart rate zone" aria-valuemin={0} aria-valuemax={5} aria-valuenow={zone}>
    {#each [0, 1, 2, 3, 4, 5] as i (i)}
      <div
        class="seg"
        class:current={active && i === zone}
        class:lower={active && i < zone}
        style:left="{pos(edges[i])}%"
        style:width="calc({pos(edges[i + 1]) - pos(edges[i])}% - 2px)"
      ></div>
      <span class="num mono" style:left="{pos(edges[i])}%" class:on={active && i === zone}>{i}</span>
    {/each}
  </div>
  <p class="readout mono">
    {#if active && pct != null}
      zone {zone} · {zoneName(zone).toLowerCase()} · {Math.round(pct)}% of {maxHr}
    {:else}
      zone --
    {/if}
  </p>
</div>

<style>
  .zone {
    display: grid;
    grid-template-columns: 1fr auto;
    align-items: start;
    gap: var(--s5);
  }
  .track {
    position: relative;
    height: 4px;
    margin-top: 6px;
    margin-bottom: 18px;
  }
  .seg {
    position: absolute;
    top: 0;
    height: 4px;
    border-radius: var(--r-pill);
    background: var(--slate-3);
    transition: background 300ms;
  }
  .seg.lower {
    background: color-mix(in srgb, var(--pink) 35%, transparent);
  }
  .seg.current {
    background: var(--pink);
  }
  .num {
    position: absolute;
    top: 10px;
    font-size: 10.5px;
    color: var(--muted);
  }
  .num.on {
    color: var(--pink);
  }
  .readout {
    font-size: 12px;
    color: var(--muted);
    white-space: nowrap;
  }
</style>
