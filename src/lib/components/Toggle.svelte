<script lang="ts">
  let {
    checked = false,
    label,
    hint = "",
    disabled = false,
    onchange,
    id = `t-${Math.random().toString(36).slice(2, 9)}`,
  }: {
    checked?: boolean;
    label: string;
    hint?: string;
    disabled?: boolean;
    onchange: (v: boolean) => void;
    id?: string;
  } = $props();
</script>

<div class="toggle-row" class:disabled>
  <div class="text">
    <label for={id}>{label}</label>
    {#if hint}<p class="hint" id="{id}-hint">{hint}</p>{/if}
  </div>
  <button
    {id}
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    aria-describedby={hint ? `${id}-hint` : undefined}
    {disabled}
    class="switch"
    onclick={() => onchange(!checked)}
  >
    <span class="knob"></span>
  </button>
</div>

<style>
  .toggle-row {
    display: flex;
    align-items: center;
    gap: 16px;
    justify-content: space-between;
    min-height: 34px;
  }
  .toggle-row.disabled {
    opacity: 0.55;
  }
  label {
    font-weight: 560;
    cursor: pointer;
  }
  .switch {
    flex: none;
    appearance: none;
    position: relative;
    width: 40px;
    height: 23px;
    border-radius: 999px;
    border: 1px solid var(--border);
    background: var(--surface-3);
    cursor: pointer;
    transition: background 0.18s;
    padding: 0;
  }
  .switch[aria-checked="true"] {
    background: var(--accent);
    border-color: transparent;
  }
  .knob {
    position: absolute;
    top: 2px;
    left: 2px;
    width: 17px;
    height: 17px;
    border-radius: 50%;
    background: #fff;
    box-shadow: 0 1px 3px rgba(0, 0, 0, 0.35);
    transition: transform 0.18s;
  }
  .switch[aria-checked="true"] .knob {
    transform: translateX(17px);
  }
  .switch:disabled {
    cursor: not-allowed;
  }
</style>
