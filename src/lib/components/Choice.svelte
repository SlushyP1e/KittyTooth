<script lang="ts" generics="T extends string">
  // Single choice among a few options: mono chips (default) or underlined mono tabs.
  // The selection is drawn by one thumb that springs between options.
  import { spring } from "../motion";

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

  let group: HTMLDivElement | undefined = $state();
  let thumb: HTMLSpanElement | undefined = $state();

  const sx = spring(0, (v) => thumb && (thumb.style.translate = `${v}px ${sy.value}px`), { k: 380, d: 26 });
  const sy = spring(0, (v) => thumb && (thumb.style.translate = `${sx.value}px ${v}px`), { k: 380, d: 26 });
  const sw = spring(0, (v) => thumb && (thumb.style.width = `${v}px`), { k: 380, d: 26 });
  let placed = false;

  function place() {
    const b = group?.querySelector<HTMLElement>('[aria-checked="true"]');
    if (!b || !thumb) return;
    const x = b.offsetLeft;
    const y = variant === "tabs" ? b.offsetTop + b.offsetHeight - 2 : b.offsetTop;
    const w = b.offsetWidth;
    thumb.style.height = variant === "tabs" ? "2px" : `${b.offsetHeight}px`;
    thumb.style.opacity = "1";
    if (!placed || !b.offsetWidth) {
      sx.snap(x);
      sy.snap(y);
      sw.snap(w);
      placed = w > 0;
    } else {
      sx.set(x);
      sy.set(y);
      sw.set(w);
    }
  }

  $effect(() => {
    void value;
    void options;
    queueMicrotask(place);
  });

  $effect(() => {
    if (!group || typeof ResizeObserver === "undefined") return;
    const ro = new ResizeObserver(() => {
      placed = false;
      place();
    });
    ro.observe(group);
    return () => ro.disconnect();
  });

  function onkeydown(e: KeyboardEvent) {
    const i = options.findIndex((o) => o.value === value);
    let n = i;
    if (e.key === "ArrowRight" || e.key === "ArrowDown") n = (i + 1) % options.length;
    else if (e.key === "ArrowLeft" || e.key === "ArrowUp") n = (i - 1 + options.length) % options.length;
    else return;
    e.preventDefault();
    onchange(options[n].value);
    queueMicrotask(() => (group?.querySelectorAll("button")[n] as HTMLButtonElement | undefined)?.focus());
  }
</script>

<div class="{variant} glide" role="radiogroup" aria-label={label} bind:this={group}>
  <span class="thumb {variant}-thumb" bind:this={thumb} aria-hidden="true"></span>
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

<style>
  .glide {
    position: relative;
  }
  .glide :global(button) {
    position: relative;
    z-index: 1;
  }
  .thumb {
    position: absolute;
    left: 0;
    top: 0;
    width: 0;
    opacity: 0;
    pointer-events: none;
    background: var(--pink);
  }
  .chips-thumb {
    border-radius: var(--r-pill);
  }
  .tabs-thumb {
    border-radius: 1px;
  }
  /* The thumb paints the selection, so selected options drop their own fill / underline. */
  .glide :global(.chip[aria-checked="true"]) {
    background: transparent;
    transition:
      color 250ms,
      transform 300ms var(--spring);
  }
  .glide :global(.tab[aria-checked="true"]) {
    border-bottom-color: transparent;
  }
</style>
