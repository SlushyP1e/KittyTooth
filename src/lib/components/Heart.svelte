<script lang="ts">
  import { cssVar, prefersReducedMotion } from "../theme";

  // Flat monoline cat-ear heart. On each real beat (`beat` increments) it scales to 1.08 and
  // back in 160 ms with a fading 6px glow. Under reduced motion a small dot blinks instead.
  let {
    beat = 0,
    active = true,
    size = 96,
    label = "Heart",
  }: { beat?: number; active?: boolean; size?: number; label?: string } = $props();

  let el: SVGSVGElement | undefined = $state();
  let dot: HTMLSpanElement | undefined = $state();
  const reduced = prefersReducedMotion();

  $effect(() => {
    void beat;
    if (!active || beat === 0) return;
    if (reduced) {
      dot?.animate([{ opacity: 1 }, { opacity: 0.15 }], { duration: 160, easing: "ease-out" });
      return;
    }
    const pink = cssVar("--pink");
    el?.animate(
      [
        { transform: "scale(1)", filter: `drop-shadow(0 0 0 ${pink})` },
        { transform: "scale(1.08)", filter: `drop-shadow(0 0 6px ${pink})`, offset: 0.35 },
        { transform: "scale(1)", filter: `drop-shadow(0 0 0 transparent)` },
      ],
      { duration: 160, easing: "ease-out" },
    );
  });
</script>

<span class="wrap" class:inactive={!active} style:--size="{size}px">
  <svg bind:this={el} class="heart" viewBox="0 0 24 24" width={size} height={size} role="img" aria-label={label}>
    <path
      class="body"
      d="M4.2 8.4 4.6 3.4l3.7 2.7A5.6 5.6 0 0 1 12 8.3a5.6 5.6 0 0 1 3.7-2.2l3.7-2.7.4 5c1.5 2.8-.1 5.8-2.4 8-1.8 1.8-5.4 4.1-5.4 4.1s-3.6-2.3-5.4-4.1c-2.3-2.2-3.9-5.2-2.4-8z"
    />
  </svg>
  {#if reduced}<span bind:this={dot} class="dot" aria-hidden="true"></span>{/if}
</span>

<style>
  .wrap {
    position: relative;
    display: inline-grid;
    place-items: center;
    width: var(--size);
    height: var(--size);
  }
  .heart {
    display: block;
    transform-origin: 50% 55%;
  }
  .body {
    fill: var(--pink-soft);
    stroke: var(--pink);
    stroke-width: 1.6;
    stroke-linejoin: round;
  }
  .inactive .body {
    fill: none;
    stroke: var(--slate-3);
  }
  .dot {
    position: absolute;
    right: 0;
    top: 0;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--pink);
    opacity: 0.15;
  }
</style>
