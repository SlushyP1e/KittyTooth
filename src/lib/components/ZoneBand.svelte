<script lang="ts">
  import { ZONE_COLORS, zoneName } from "../format";

  // Six-segment zone band with a marker at the current heart rate.
  let {
    bpm,
    maxHr,
    thresholds,
    zone,
    active,
  }: { bpm: number | null; maxHr: number; thresholds: number[]; zone: number; active: boolean } = $props();

  // Display scale: 40 % .. 100 % of max HR.
  const LO = 40;
  const pos = (pct: number) => Math.max(0, Math.min(100, ((pct - LO) / (100 - LO)) * 100));
  const edges = $derived([LO, ...thresholds, 100]);
  const pct = $derived(bpm == null ? null : (bpm * 100) / maxHr);
</script>

<div class="zoneband" class:inactive={!active}>
  <div class="head">
    <span class="label">Zone</span>
    <span class="zname" style:color={active ? ZONE_COLORS[zone] : undefined}>
      {active ? `${zone} · ${zoneName(zone)}` : "--"}
    </span>
    <span class="spacer"></span>
    <span class="muted tabular">{pct == null || !active ? "--" : `${Math.round(pct)}% of max ${maxHr}`}</span>
  </div>
  <div class="bar" role="meter" aria-label="Heart rate zone" aria-valuemin={0} aria-valuemax={5} aria-valuenow={zone}>
    {#each ZONE_COLORS as c, i (i)}
      <div
        class="seg"
        class:current={active && i === zone}
        style:left="{pos(i === 0 ? LO : edges[i])}%"
        style:width="{pos(edges[i + 1]) - pos(i === 0 ? LO : edges[i])}%"
        style:background={c}
      ></div>
    {/each}
    {#if pct != null && active}
      <div class="marker" style:left="{pos(pct)}%"></div>
    {/if}
  </div>
</div>

<style>
  .zoneband {
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  .head {
    display: flex;
    align-items: baseline;
    gap: 8px;
    font-size: 13px;
  }
  .label {
    color: var(--muted);
    font-weight: 560;
  }
  .zname {
    font-weight: 650;
  }
  .spacer {
    flex: 1;
  }
  .bar {
    position: relative;
    height: 10px;
    border-radius: 999px;
    background: var(--surface-2);
  }
  .seg {
    position: absolute;
    top: 0;
    bottom: 0;
    opacity: 0.35;
    transition: opacity 0.3s;
  }
  .seg:first-child {
    border-radius: 999px 0 0 999px;
  }
  .seg:nth-child(6) {
    border-radius: 0 999px 999px 0;
  }
  .seg.current {
    opacity: 1;
  }
  .marker {
    position: absolute;
    top: -4px;
    width: 4px;
    height: 18px;
    margin-left: -2px;
    border-radius: 3px;
    background: var(--text);
    box-shadow: 0 0 0 2px var(--surface);
    transition: left 0.6s cubic-bezier(0.2, 0.8, 0.3, 1);
  }
  .inactive .seg {
    opacity: 0.18;
  }
</style>
