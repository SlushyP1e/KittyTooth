<script lang="ts">
  // A true on/off setting. Renders the standard settings row (label + help left, switch right),
  // or just the switch with `bare`.
  let {
    checked = false,
    label,
    hint = "",
    disabled = false,
    bare = false,
    onchange,
    id = `t-${Math.random().toString(36).slice(2, 9)}`,
  }: {
    checked?: boolean;
    label: string;
    hint?: string;
    disabled?: boolean;
    bare?: boolean;
    onchange: (v: boolean) => void;
    id?: string;
  } = $props();
</script>

{#snippet sw()}
  <button
    {id}
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    aria-describedby={hint && !bare ? `${id}-hint` : undefined}
    {disabled}
    class="switch"
    onclick={() => onchange(!checked)}
  >
    <span class="knob"></span>
  </button>
{/snippet}

{#if bare}
  {@render sw()}
{:else}
  <div class="srow" class:disabled>
    <div class="stext">
      <label class="slabel" for={id}>{label}</label>
      {#if hint}<p class="shelp" id="{id}-hint">{hint}</p>{/if}
    </div>
    <div class="sctrl">{@render sw()}</div>
  </div>
{/if}

<style>
  .disabled {
    opacity: 0.55;
  }
  label {
    cursor: pointer;
  }
  .switch {
    flex: none;
    appearance: none;
    position: relative;
    width: 40px;
    height: 22px;
    border-radius: var(--r-pill);
    border: 0;
    background: var(--slate-3);
    cursor: pointer;
    transition: background 220ms;
    padding: 0;
  }
  .switch[aria-checked="true"] {
    background: var(--pink-deep);
  }
  .knob {
    position: absolute;
    top: 3px;
    left: 3px;
    width: 16px;
    height: 16px;
    border-radius: 50%;
    background: var(--muted);
    transition:
      transform 220ms var(--spring),
      background 220ms;
  }
  .switch[aria-checked="true"] .knob {
    transform: translateX(18px);
    background: var(--knob-on);
  }
  .switch:disabled {
    cursor: not-allowed;
  }
</style>
