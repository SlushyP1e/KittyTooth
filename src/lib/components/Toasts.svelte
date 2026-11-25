<script lang="ts">
  import { app } from "../stores/app.svelte";
</script>

<div class="toasts" aria-live="polite" aria-relevant="additions">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}" role={t.kind === "error" ? "alert" : "status"}>
      <div class="body">
        <p class="title">{t.title}</p>
        {#if t.message}<p class="msg">{t.message}</p>{/if}
        {#if t.fix}<p class="fix">{t.fix}</p>{/if}
      </div>
      <button class="btn ghost icon small" aria-label="Dismiss" onclick={() => app.dismiss(t.id)}>✕</button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    right: 18px;
    bottom: 18px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    z-index: 50;
    max-width: min(420px, calc(100vw - 36px));
  }
  .toast {
    display: flex;
    gap: 10px;
    align-items: flex-start;
    padding: 12px 12px 12px 16px;
    border-radius: var(--radius);
    background: var(--surface);
    border: 1px solid var(--border);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.35);
    border-left: 4px solid var(--busy);
  }
  .toast.success {
    border-left-color: var(--ok);
  }
  .toast.error {
    border-left-color: var(--error);
  }
  .body {
    flex: 1;
  }
  .title {
    font-weight: 650;
  }
  .msg,
  .fix {
    color: var(--muted);
    font-size: 13px;
  }
</style>
