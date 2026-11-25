<script lang="ts">
  import { app } from "../stores/app.svelte";
</script>

<div class="toasts" aria-live="polite" aria-relevant="additions">
  {#each app.toasts as t (t.id)}
    <div class="toast {t.kind}" role={t.kind === "error" ? "alert" : "status"}>
      <div class="body">
        <p class="title">{t.title}</p>
        {#if t.message}<p class="msg">{t.message}</p>{/if}
        {#if t.fix}<p class="msg">{t.fix}</p>{/if}
      </div>
      <button class="btn quiet icon small" aria-label="Dismiss" onclick={() => app.dismiss(t.id)}>
        <svg viewBox="0 0 12 12" width="10" height="10" aria-hidden="true"
          ><path d="M2 2l8 8M10 2l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" /></svg
        >
      </button>
    </div>
  {/each}
</div>

<style>
  .toasts {
    position: fixed;
    left: 50%;
    bottom: var(--s5);
    transform: translateX(-50%);
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: var(--s2);
    z-index: 50;
    width: min(440px, calc(100vw - 32px));
  }
  .toast {
    display: flex;
    gap: var(--s3);
    align-items: flex-start;
    width: 100%;
    padding: var(--s3) var(--s3) var(--s3) var(--s4);
    border-radius: var(--r-row);
    background: var(--slate-0);
    border: 1px solid var(--slate-3);
    animation: up 220ms ease-out both;
  }
  .toast.error {
    box-shadow: inset 2px 0 0 var(--red);
  }
  .toast.success {
    box-shadow: inset 2px 0 0 var(--green);
  }
  @keyframes up {
    from {
      opacity: 0;
      transform: translateY(12px);
    }
    to {
      opacity: 1;
      transform: none;
    }
  }
  .body {
    flex: 1;
  }
  .title {
    color: var(--text);
    font-weight: 600;
  }
  .msg {
    color: var(--muted);
    font-size: 12.5px;
  }
</style>
