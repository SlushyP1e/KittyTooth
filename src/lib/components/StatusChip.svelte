<script lang="ts">
  import type { StatusView } from "../format";
  let { status }: { status: StatusView } = $props();
  const cls = $derived({ ok: "ok", warn: "warn", error: "err", busy: "neutral busy", off: "neutral" }[status.tone]);
</script>

<span class="tag {cls}" role="status" aria-live="polite"><span class="lbl">{status.label}</span></span>

<style>
  .lbl {
    text-transform: lowercase;
  }
  .busy::before {
    content: "";
    width: 6px;
    height: 6px;
    border-radius: 50%;
    background: currentColor;
    animation: blink 1.1s ease-in-out infinite;
  }
  @keyframes blink {
    50% {
      opacity: 0.25;
    }
  }
</style>
