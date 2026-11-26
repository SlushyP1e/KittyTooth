<script lang="ts">
  import type { AlertStatus } from "../types";

  let { alert, onsettings }: { alert: AlertStatus; onsettings?: () => void } = $props();

  const high = $derived(alert.kind === "high");
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 1000);
    return () => clearInterval(t);
  });
  const mins = $derived(Math.max(0, Math.floor((now - alert.since_ms) / 60_000)));
</script>

<div class="alert" class:low={!high} role="alert" data-testid="hr-alert">
  <span class="pulse" aria-hidden="true"></span>
  <div class="body">
    <p class="title">
      {high ? "High heart rate" : "Low heart rate"}{#if alert.test}<span class="tag neutral mono">test</span>{/if}
    </p>
    <p class="msg mono">
      <span class="tabular">{alert.bpm}</span> bpm · {high ? "above" : "below"} your {alert.threshold} bpm alert{#if mins >= 1}
        · {mins} min{/if}
    </p>
  </div>
  {#if onsettings}<button class="btn small" onclick={onsettings}>Alert settings</button>{/if}
</div>

<style>
  .alert {
    --tone: var(--red);
    display: flex;
    align-items: center;
    gap: var(--s4);
    padding: var(--s3) var(--s4);
    border-radius: var(--r-row);
    background: color-mix(in srgb, var(--tone) 12%, var(--card));
    border: 1px solid color-mix(in srgb, var(--tone) 40%, transparent);
    margin-bottom: var(--s4);
    animation: alert-in 420ms var(--spring) both;
  }
  .alert.low {
    --tone: var(--amber);
  }
  .pulse {
    width: 10px;
    height: 10px;
    border-radius: 50%;
    background: var(--tone);
    flex: none;
    animation: alert-pulse 1.2s ease-out infinite;
  }
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .title {
    color: var(--text);
    font-weight: 600;
    display: flex;
    align-items: center;
    gap: var(--s2);
  }
  .msg {
    color: var(--body);
    font-size: 12.5px;
  }
  @keyframes alert-in {
    from {
      opacity: 0;
      transform: translateY(-8px) scale(0.98);
    }
  }
  @keyframes alert-pulse {
    0% {
      box-shadow: 0 0 0 0 color-mix(in srgb, var(--tone) 60%, transparent);
    }
    100% {
      box-shadow: 0 0 0 9px transparent;
    }
  }
</style>
