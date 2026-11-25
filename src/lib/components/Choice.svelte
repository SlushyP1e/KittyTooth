<script lang="ts" generics="T extends string">
  // Single choice among a few options: mono chips (default) or underlined mono tabs.
  let {
    value,
    options,
    label,
    variant = "chips",
    onchange,
  }: {
    value: T;
    options: { value: T; label: string }[];
    label: string;
    variant?: "chips" | "tabs";
    onchange: (v: T) => void;
  } = $props();

  function onkeydown(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    let n = i;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") n = (i + 1) % options.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") n = (i - 1 + options.length) % options.length;
    else return;
    e.preventDefault();
    onchange(options[n].value);
    const group = (e.currentTarget as HTMLElement).parentElement;
    queueMicrotask(() => (group?.querySelectorAll("button")[n] as HTMLButtonElement | undefined)?.focus());
  }
</script>

<div class={variant} role="radiogroup" aria-label={label}>
  {#each options as o (o.value)}
    <button
      type="button"
      class={variant === "chips" ? "chip" : "tab"}
      role="radio"
      aria-checked={o.value === value}
      tabindex={o.value === value ? 0 : -1}
      onclick={() => onchange(o.value)}
      {onkeydown}>{o.label}</button
    >
  {/each}
</div>
