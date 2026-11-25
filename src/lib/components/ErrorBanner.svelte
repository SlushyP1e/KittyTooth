<script lang="ts">
  import type { Snippet } from "svelte";
  import type { UserError } from "../types";
  let { error, actions }: { error: UserError; actions?: Snippet } = $props();
</script>

<div class="err" role="alert">
  <div class="body">
    <p class="title">{error.title}</p>
    <p class="msg">{error.message}</p>
    <p class="fix">Try this: {error.fix}</p>
  </div>
  {#if actions}<div class="actions">{@render actions()}</div>{/if}
</div>

<style>
  .err {
    display: flex;
    gap: var(--s4);
    align-items: center;
    padding: var(--s3) var(--s4);
    border-radius: var(--r-row);
    background: var(--slate-2);
    box-shadow: inset 2px 0 0 var(--red);
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
  }
  .msg {
    color: var(--body);
  }
  .fix {
    color: var(--muted);
    font-size: 12.5px;
  }
  .actions {
    display: flex;
    gap: var(--s2);
  }
</style>
