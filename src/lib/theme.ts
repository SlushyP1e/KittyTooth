import type { Theme } from "./types";
import { readableOn } from "./format";

export const ACCENTS = ["#ff6fa8", "#b77dff", "#6c9dff", "#3fcfb4", "#ffb057", "#ff6b6b"] as const;

/** Resolves "system" to the OS preference. */
export function resolveTheme(theme: Theme): "dark" | "light" {
  if (theme !== "system") return theme;
  const mq = typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: light)");
  return mq && mq.matches ? "light" : "dark";
}

export function applyTheme(theme: Theme, accent: string, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = resolveTheme(theme);
  const color = /^#[0-9a-f]{6}$/i.test(accent) ? accent : ACCENTS[0];
  root.style.setProperty("--accent", color);
  root.style.setProperty("--accent-text", readableOn(color));
}

export function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}
