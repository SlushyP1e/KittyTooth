<script lang="ts">
  // The cat-head outline. With `spark`, a glowing accent comet periodically runs from the left ear
  // tip, around the chin, and up to the right ear, where it lands as a small orb that pops and fades.
  let { size = 22, spark = false, period = 9 }: { size?: number; spark?: boolean; period?: number } = $props();

  const OUTLINE =
    "M4.2 8.4 4.6 3.4l3.7 2.7A5.6 5.6 0 0 1 12 8.3a5.6 5.6 0 0 1 3.7-2.2l3.7-2.7.4 5c1.5 2.8-.1 5.8-2.4 8-1.8 1.8-5.4 4.1-5.4 4.1s-3.6-2.3-5.4-4.1c-2.3-2.2-3.9-5.2-2.4-8z";
  // Same outline from the right ear tip, down and round the chin, to the left ear tip (ears' inner
  // edges excluded). The comet travels it backwards: left ear -> chin -> right ear.
  const TRACE =
    "M19.4 3.4 19.8 8.4C21.3 11.2 19.7 14.2 17.4 16.4 15.6 18.2 12 20.5 12 20.5S8.4 18.2 6.6 16.4C4.3 14.2 2.7 11.2 4.2 8.4L4.6 3.4";
</script>

<svg
  viewBox="0 0 24 24"
  width={size}
  height={size}
  aria-hidden="true"
  class:spark
  style="--period: {period}s"
  overflow="visible"
>
  <path class="outline" d={OUTLINE} />
  {#if spark}
    <path class="trail" d={TRACE} pathLength="100" />
    <path class="head" d={TRACE} pathLength="100" />
    <circle class="orb" cx="19.4" cy="3.4" r="1.9" />
  {/if}
</svg>

<style>
  svg {
    overflow: visible;
  }
  path {
    fill: none;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .outline {
    stroke: var(--pink);
    stroke-width: 1.8;
  }

  /* Comet: a long faint tail and a short bright head share one leading edge. Dash positions are in
     pathLength units (0 = right ear, 100 = left ear); the leading edge sits at -dashoffset. */
  .trail,
  .head {
    stroke-dashoffset: -100;
    animation: comet var(--period) cubic-bezier(0.45, 0, 0.3, 1) infinite;
  }
  .trail {
    stroke: var(--pink);
    stroke-width: 2.6;
    stroke-dasharray: 34 200;
    opacity: 0.55;
    filter: drop-shadow(0 0 1.5px var(--pink));
  }
  .head {
    stroke: var(--spark);
    stroke-width: 2.4;
    stroke-dasharray: 9 200;
    filter: drop-shadow(0 0 2px var(--pink));
  }
  @keyframes comet {
    0% {
      stroke-dashoffset: -100;
      opacity: 0;
    }
    1% {
      opacity: 1;
    }
    13% {
      stroke-dashoffset: 0;
      opacity: 1;
    }
    /* Head reaches the right ear tip, then the dash drains into it. */
    16% {
      stroke-dashoffset: 34;
      opacity: 0;
    }
    100% {
      stroke-dashoffset: 34;
      opacity: 0;
    }
  }
  .trail {
    animation-name: comet-trail;
  }
  @keyframes comet-trail {
    0% {
      stroke-dashoffset: -100;
      opacity: 0;
    }
    1% {
      opacity: 0.55;
    }
    13% {
      stroke-dashoffset: 0;
      opacity: 0.55;
    }
    16% {
      stroke-dashoffset: 34;
      opacity: 0;
    }
    100% {
      stroke-dashoffset: 34;
      opacity: 0;
    }
  }

  /* Orb: springs in at the right ear as the comet arrives, glows briefly, then fades. */
  .orb {
    fill: var(--spark);
    filter: drop-shadow(0 0 2.5px var(--pink));
    transform-box: fill-box;
    transform-origin: center;
    animation: orb var(--period) infinite;
    opacity: 0;
  }
  @keyframes orb {
    0%,
    12.5% {
      opacity: 0;
      transform: scale(0);
      animation-timing-function: cubic-bezier(0.34, 1.56, 0.64, 1);
    }
    16% {
      opacity: 1;
      transform: scale(1.35);
    }
    19% {
      transform: scale(1);
      animation-timing-function: ease-in;
    }
    28% {
      opacity: 0;
      transform: scale(0.6);
    }
    100% {
      opacity: 0;
      transform: scale(0.6);
    }
  }
</style>
