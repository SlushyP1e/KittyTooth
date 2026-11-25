<script lang="ts">
  let { label, value, unit = "", hint = "" }: { label: string; value: string; unit?: string; hint?: string } = $props();
</script>

<div class="tile" title={hint || undefined}>
  <span class="label">{label}</span>
  <span class="value tabular">{value}{#if unit && value !== "--"}<span class="unit">{unit}</span>{/if}</span>
</div>

<style>
  .tile {
    display: flex;
    flex-direction: column;
    gap: 2px;
    padding: 14px 16px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: var(--shadow);
    min-width: 0;
  }
  .label {
    font-size: 12.5px;
    color: var(--muted);
    font-weight: 560;
  }
  .value {
    font-family: var(--font-display);
    font-size: 26px;
    font-weight: 650;
    letter-spacing: -0.02em;
    white-space: nowrap;
  }
  .unit {
    font-size: 13px;
    color: var(--muted);
    font-weight: 560;
    margin-left: 4px;
  }
</style>
