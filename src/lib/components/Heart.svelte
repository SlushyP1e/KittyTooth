<script lang="ts">
  import { prefersReducedMotion } from "../theme";

  // A heart with little cat ears that pulses on each real beat (`beat` increments per beat).
  let {
    beat = 0,
    interval = 800,
    active = true,
    size = 120,
    label = "Heart",
  }: { beat?: number; interval?: number; active?: boolean; size?: number; label?: string } = $props();

  let el: SVGSVGElement | undefined = $state();
  const reduced = prefersReducedMotion();

  $effect(() => {
    // Re-run on every beat.
    void beat;
    if (!el || !active || beat === 0) return;
    const dur = Math.max(160, Math.min(360, interval * 0.55));
    if (reduced) {
      el.animate([{ opacity: 1 }, { opacity: 0.78 }, { opacity: 1 }], { duration: dur, easing: "ease-out" });
    } else {
      el.animate(
        [
          { transform: "scale(1)" },
          { transform: "scale(1.13)", offset: 0.22 },
          { transform: "scale(0.98)", offset: 0.55 },
          { transform: "scale(1)" },
        ],
        { duration: dur, easing: "cubic-bezier(.2,.8,.3,1)" },
      );
    }
  });
</script>

<svg
  bind:this={el}
  class="heart"
  class:inactive={!active}
  viewBox="0 0 24 24"
  width={size}
  height={size}
  role="img"
  aria-label={label}
>
  <defs>
    <linearGradient id="kt-heart-grad" x1="0" y1="0" x2="0" y2="1">
      <stop offset="0" stop-color="color-mix(in srgb, var(--accent) 80%, white)" />
      <stop offset="1" stop-color="var(--accent)" />
    </linearGradient>
  </defs>
  <path
    fill="url(#kt-heart-grad)"
    d="M3.4 8.1 3.9 2.6l4.3 3.1A6.1 6.1 0 0 1 12 8c1.1-1.2 2.3-2 3.8-2.3l4.3-3.1.5 5.5c1.6 3-.1 6.2-2.6 8.6C16 18.6 12 21.2 12 21.2s-4-2.6-6-4.5C3.5 14.3 1.8 11.1 3.4 8.1z"
  />
  <path class="ear-inner" d="M4.6 6.6l.3-2.4 1.9 1.4z M19.4 6.6l-.3-2.4-1.9 1.4z" />
  <path class="shine" d="M6.3 9.1c.4-1.3 1.6-2.1 2.9-2" />
</svg>

<style>
  .heart {
    display: block;
    transform-origin: 50% 58%;
    filter: drop-shadow(0 6px 18px color-mix(in srgb, var(--accent) 35%, transparent));
    transition:
      filter 0.3s,
      opacity 0.3s;
  }
  .heart.inactive {
    opacity: 0.35;
    filter: grayscale(0.8);
  }
  .ear-inner {
    fill: rgba(255, 255, 255, 0.35);
  }
  .shine {
    fill: none;
    stroke: rgba(255, 255, 255, 0.55);
    stroke-width: 0.9;
    stroke-linecap: round;
  }
</style>
