<script lang="ts">
  import type { Snippet } from "svelte";
  import type { UserError } from "../types";
  let { error, actions }: { error: UserError; actions?: Snippet } = $props();
</script>

<div class="banner" role="alert">
  <svg viewBox="0 0 24 24" width="22" height="22" aria-hidden="true" class="icon">
    <circle cx="12" cy="12" r="10" fill="none" stroke="currentColor" stroke-width="2" />
    <path d="M12 7v6M12 16.5v.5" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" />
  </svg>
  <div class="body">
    <p class="title">{error.title}</p>
    <p class="msg">{error.message}</p>
    <p class="fix"><strong>Try this:</strong> {error.fix}</p>
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .banner {
    display: flex;
    gap: 12px;
    align-items: flex-start;
    padding: 14px 16px;
    border-radius: var(--radius);
    border: 1px solid color-mix(in srgb, var(--error) 40%, var(--border));
    background: color-mix(in srgb, var(--error) 9%, var(--surface));
  }
  .icon {
    color: var(--error);
    flex: none;
    margin-top: 1px;
  }
  .body {
    flex: 1;
    display: flex;
    flex-direction: column;
    gap: 3px;
  }
  .title {
    font-weight: 650;
  }
  .msg {
    color: var(--text);
  }
  .fix {
    color: var(--muted);
  }
  .actions {
    display: flex;
    gap: 8px;
    align-self: center;
  }
</style>
