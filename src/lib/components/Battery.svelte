<script lang="ts">
  let { level }: { level: number | null } = $props();
  const pct = $derived(level == null ? 0 : Math.max(0, Math.min(100, level)));
  const low = $derived(level != null && level <= 15);
</script>

{#if level != null}
  <span class="battery" class:low title="Sensor battery {pct}%">
    <svg viewBox="0 0 26 14" width="24" height="13" aria-hidden="true">
      <rect x="0.75" y="0.75" width="21.5" height="12.5" rx="3" fill="none" stroke="currentColor" stroke-width="1.5" />
      <rect x="23.2" y="4.5" width="2" height="5" rx="1" fill="currentColor" />
      <rect x="2.6" y="2.6" width={(17.8 * pct) / 100} height="8.8" rx="1.6" class="fill" />
    </svg>
    <span class="tabular">{pct}%</span>
    <span class="sr-only">battery</span>
  </span>
{/if}

<style>
  .battery {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    color: var(--muted);
    font-size: 12.5px;
    font-weight: 600;
  }
  .fill {
    fill: var(--ok);
  }
  .low .fill {
    fill: var(--error);
  }
  .low {
    color: var(--error);
  }
</style>
