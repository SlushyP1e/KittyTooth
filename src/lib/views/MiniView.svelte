<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Heart from "../components/Heart.svelte";
  import { formatBpm } from "../format";

  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
</script>

<!-- Compact always-on-top window: drag anywhere, double-click to open the main window. -->
<div
  class="mini"
  role="presentation"
  data-tauri-drag-region
  ondblclick={() => app.run((a) => a.showMain())}
>
  <div data-tauri-drag-region class="heart">
    <Heart beat={app.beatCount} interval={app.beatInterval} active={live} size={56} label="Heartbeat" />
  </div>
  <div data-tauri-drag-region class="num tabular" data-testid="mini-bpm">
    {live ? formatBpm(s.bpm) : "--"}<span class="unit">bpm</span>
  </div>
  <button class="close" aria-label="Close mini window" onclick={() => app.run((a) => a.closeMini())}>✕</button>
</div>

<style>
  .mini {
    position: relative;
    height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 10px;
    padding: 0 16px;
    border-radius: 18px;
    background: color-mix(in srgb, var(--surface) 88%, transparent);
    border: 1px solid var(--border);
    user-select: none;
    cursor: grab;
  }
  .num {
    font-family: var(--font-display);
    font-size: 46px;
    font-weight: 700;
    letter-spacing: -0.03em;
    min-width: 3ch;
  }
  .unit {
    font-size: 14px;
    color: var(--muted);
    margin-left: 4px;
    font-weight: 600;
  }
  .close {
    position: absolute;
    top: 6px;
    right: 8px;
    border: 0;
    background: transparent;
    color: var(--muted);
    font-size: 12px;
    cursor: pointer;
    opacity: 0;
    transition: opacity 0.15s;
    border-radius: 6px;
    width: 22px;
    height: 22px;
  }
  .mini:hover .close,
  .close:focus-visible {
    opacity: 1;
  }
</style>
