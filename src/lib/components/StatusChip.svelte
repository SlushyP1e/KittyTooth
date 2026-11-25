<script lang="ts">
  import type { StatusView } from "../format";
  let { status }: { status: StatusView } = $props();
</script>

<span class="chip {status.tone}" role="status" aria-live="polite">
  <span class="dot" aria-hidden="true"></span>
  {status.label}
</span>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 7px;
    height: 26px;
    padding: 0 11px 0 9px;
    border-radius: 999px;
    font-size: 12.5px;
    font-weight: 600;
    background: var(--surface-2);
    border: 1px solid var(--border);
    color: var(--text);
    white-space: nowrap;
  }
  .dot {
    width: 8px;
    height: 8px;
    border-radius: 50%;
    background: var(--faint);
  }
  .ok .dot {
    background: var(--ok);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--ok) 25%, transparent);
  }
  .warn .dot {
    background: var(--warn);
  }
  .error .dot {
    background: var(--error);
  }
  .busy .dot {
    background: var(--busy);
    animation: blink 1.1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.3;
    }
  }
</style>
