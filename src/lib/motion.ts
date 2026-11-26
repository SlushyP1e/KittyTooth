// Spring motion shared by the UI (same engine as the SlushyLoader hub): one damped spring per
// animated value, all stepped by a single requestAnimationFrame loop that stops when everything rests.
import { prefersReducedMotion } from "./theme";

export interface Spring {
  readonly value: number;
  /** Animate towards `target`. */
  set(target: number): void;
  /** Jump to `target` with no motion. */
  snap(target: number): void;
  /** Give the spring a velocity kick (e.g. for a squish). */
  kick(from: number): void;
}

interface Internal {
  x: number;
  v: number;
  target: number;
  k: number;
  d: number;
  onUpdate: (v: number) => void;
}

const active = new Set<Internal>();
let running = false;
let last = 0;

function step(t: number) {
  const dt = Math.min(0.032, (t - last) / 1000);
  last = t;
  for (const s of active) {
    const a = -s.k * (s.x - s.target) - s.d * s.v;
    s.v += a * dt;
    s.x += s.v * dt;
    if (Math.abs(s.v) < 0.01 && Math.abs(s.x - s.target) < 0.01) {
      s.x = s.target;
      s.v = 0;
      active.delete(s);
    }
    s.onUpdate(s.x);
  }
  if (active.size) requestAnimationFrame(step);
  else running = false;
}

function start() {
  if (running || typeof requestAnimationFrame === "undefined") return;
  running = true;
  last = performance.now();
  requestAnimationFrame(step);
}

/**
 * Creates a spring. Defaults match the hub: snappy with a small overshoot, settling in ~0.3 s.
 * Honors prefers-reduced-motion by snapping.
 */
export function spring(
  initial: number,
  onUpdate: (v: number) => void,
  { k = 420, d = 30 } = {},
): Spring {
  const s: Internal = { x: initial, v: 0, target: initial, k, d, onUpdate };
  return {
    get value() {
      return s.x;
    },
    set(target) {
      s.target = target;
      if (
        prefersReducedMotion() ||
        typeof requestAnimationFrame === "undefined"
      ) {
        this.snap(target);
        return;
      }
      active.add(s);
      start();
    },
    snap(target) {
      s.x = s.target = target;
      s.v = 0;
      active.delete(s);
      onUpdate(target);
    },
    kick(from) {
      if (prefersReducedMotion()) return;
      s.x = from;
      active.add(s);
      start();
    },
  };
}

/** Hub easing for one-shot WAAPI animations (small overshoot). */
export const SPRING_EASE = "cubic-bezier(.34,1.4,.64,1)";

/**
 * Svelte action: staggers the direct children of a page in from below on mount, like the hub's
 * page switch. Only runs once per mount (pages remount on navigation via {#key}).
 */
export function stagger(node: HTMLElement, { dir = 1 }: { dir?: number } = {}) {
  if (prefersReducedMotion() || typeof node.animate !== "function") return;
  const page = node.querySelector(":scope > .page") ?? node;
  [...page.children].forEach((c, i) =>
    (c as HTMLElement).animate(
      [
        { opacity: 0, transform: `translateY(${14 * dir}px) scale(.985)` },
        { opacity: 1, transform: "none" },
      ],
      {
        duration: 460,
        delay: 40 + i * 45,
        easing: SPRING_EASE,
        fill: "backwards",
      },
    ),
  );
}
