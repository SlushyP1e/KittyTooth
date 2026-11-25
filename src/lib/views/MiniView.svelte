<script lang="ts">
  import { app } from "../stores/app.svelte";
  import Heart from "../components/Heart.svelte";
  import { formatBpm } from "../format";

  const s = $derived(app.snapshot);
  const live = $derived(s.data_valid || s.test_sweep);
</script>

<!-- Compact window: drag anywhere, double-click to open the main window. -->
<div class="mini" role="presentation" data-tauri-drag-region ondblclick={() => app.run((a) => a.showMain())}>
  <div class="pill" data-tauri-drag-region>
    <Heart beat={app.beatCount} active={live} size={40} label="Heartbeat" />
    <span class="num tabular" data-tauri-drag-region data-testid="mini-bpm">{live ? formatBpm(s.bpm) : "--"}</span>
    <span class="unit mono" data-tauri-drag-region>bpm</span>
  </div>
  <button class="close" aria-label="Close mini window" onclick={() => app.run((a) => a.closeMini())}>
    <svg viewBox="0 0 12 12" width="9" height="9" aria-hidden="true"
      ><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
    >
  </button>
</div>

<style>
  .mini {
    position: relative;
    height: 100vh;
    display: grid;
    place-items: center;
    user-select: none;
    cursor: grab;
  }
  .pill {
    display: flex;
    align-items: center;
    gap: 10px;
    padding: 10px 18px 10px 14px;
    border-radius: var(--r-panel);
    background: color-mix(in srgb, var(--slate-0) 80%, transparent);
  }
  .num {
    font-family: var(--display);
    font-weight: 800;
    font-size: 44px;
    line-height: 1;
    letter-spacing: -0.03em;
    color: var(--text);
    min-width: 2.4ch;
  }
  .unit {
    font-size: 12px;
    color: var(--muted);
    align-self: flex-end;
    margin-bottom: 6px;
  }
  .close {
    position: absolute;
    top: 6px;
    right: 6px;
    width: 20px;
    height: 20px;
    display: grid;
    place-items: center;
    border: 0;
    border-radius: var(--r-pill);
    background: var(--slate-2);
    color: var(--muted);
    cursor: pointer;
    opacity: 0;
    transition: opacity 120ms;
  }
  .mini:hover .close,
  .close:focus-visible {
    opacity: 1;
  }
</style>
