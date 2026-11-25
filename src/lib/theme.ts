import type { Theme } from "./types";

/** Named accent presets; their colors live in tokens.css (`[data-accent=...]`). */
export const ACCENT_PRESETS = ["pink", "peach", "lilac", "sky"] as const;
export type AccentPreset = (typeof ACCENT_PRESETS)[number];
export const DEFAULT_ACCENT: AccentPreset = "pink";

const HEX = /^#[0-9a-f]{6}$/i;

export function isPreset(v: string): v is AccentPreset {
  return (ACCENT_PRESETS as readonly string[]).includes(v);
}

export function isCustomAccent(v: string): boolean {
  return HEX.test(v);
}

/** Resolves "system" to the OS preference. */
export function resolveTheme(theme: Theme): "dark" | "light" {
  if (theme !== "system") return theme;
  const mq = typeof window !== "undefined" && window.matchMedia?.("(prefers-color-scheme: light)");
  return mq && mq.matches ? "light" : "dark";
}

/**
 * Applies theme and accent. Presets switch `data-accent` (tokens.css swaps --pink, --pink-deep and
 * --pink-soft together). A custom hex sets --pink and derives the other two from it.
 */
export function applyTheme(theme: Theme, accent: string, root: HTMLElement = document.documentElement): void {
  root.dataset.theme = resolveTheme(theme);
  for (const p of ["--pink", "--pink-deep", "--pink-soft", "--on-pink"]) root.style.removeProperty(p);
  if (isCustomAccent(accent)) {
    root.dataset.accent = "custom";
    root.style.setProperty("--pink", accent);
    root.style.setProperty("--pink-deep", `color-mix(in srgb, ${accent} 68%, black)`);
    root.style.setProperty("--pink-soft", `color-mix(in srgb, ${accent} 12%, transparent)`);
    root.style.setProperty("--on-pink", readableOn(accent));
  } else {
    root.dataset.accent = isPreset(accent) ? accent : DEFAULT_ACCENT;
  }
}

/** Text color with the higher WCAG contrast on a custom accent. */
export function readableOn(hex: string): "black" | "white" {
  const m = HEX.exec(hex);
  if (!m) return "white";
  const n = parseInt(hex.slice(1), 16);
  const ch = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((c) => {
    const x = c / 255;
    return x <= 0.03928 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
  });
  const lum = 0.2126 * ch[0] + 0.7152 * ch[1] + 0.0722 * ch[2];
  return (lum + 0.05) / 0.05 >= 1.05 / (lum + 0.05) ? "black" : "white";
}

export function prefersReducedMotion(): boolean {
  return typeof window !== "undefined" && !!window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
}

/** Reads a CSS custom property from the document root (e.g. for canvas drawing). */
export function cssVar(name: string): string {
  if (typeof document === "undefined") return "";
  return getComputedStyle(document.documentElement).getPropertyValue(name).trim();
}

/** Converts a computed CSS color (#rrggbb or rgb()/rgba()) to an rgba() string with alpha. */
export function withAlpha(color: string, alpha: number): string {
  const hex = /^#([0-9a-f]{6})$/i.exec(color);
  if (hex) {
    const n = parseInt(hex[1], 16);
    return `rgba(${(n >> 16) & 255}, ${(n >> 8) & 255}, ${n & 255}, ${alpha})`;
  }
  const rgb = /^rgba?\(([^)]+)\)$/i.exec(color);
  if (rgb) {
    const [r, g, b] = rgb[1].split(/[\s,/]+/).filter(Boolean);
    return `rgba(${r}, ${g}, ${b}, ${alpha})`;
  }
  return color;
}
