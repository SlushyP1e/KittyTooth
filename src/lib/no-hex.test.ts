import { describe, expect, it } from "vitest";
import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

// Design-system guard: literal colors may only live in src/tokens.css.
const ROOT = join(__dirname, "..");
const ALLOWED = new Set(["tokens.css"]);
const HEX = /#(?:[0-9a-fA-F]{8}|[0-9a-fA-F]{6}|[0-9a-fA-F]{4}|[0-9a-fA-F]{3})(?![0-9a-zA-Z_-])/g;
const FUNC = /\b(?:rgba?|hsla?)\(\s*\d/g;
const BANNED_FONTS = /\b(?:Inter|Roboto|system-ui|Segoe UI)\b/g;

function walk(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) return name === "assets" ? [] : walk(p);
    return /\.(svelte|ts|css)$/.test(name) && !/\.test\.ts$/.test(name) ? [p] : [];
  });
}

describe("design tokens", () => {
  const files = walk(ROOT).filter((f) => !ALLOWED.has(relative(ROOT, f)));

  it("has no hardcoded colors outside tokens.css", () => {
    const hits: string[] = [];
    for (const f of files) {
      readFileSync(f, "utf8")
        .split("\n")
        .forEach((line, i) => {
          for (const m of line.matchAll(HEX)) hits.push(`${relative(ROOT, f)}:${i + 1} ${m[0]}`);
          for (const m of line.matchAll(FUNC)) hits.push(`${relative(ROOT, f)}:${i + 1} ${m[0]}`);
        });
    }
    expect(hits).toEqual([]);
  });

  it("uses only the bundled type families", () => {
    const hits = files.flatMap((f) =>
      [...readFileSync(f, "utf8").matchAll(BANNED_FONTS)].map((m) => `${relative(ROOT, f)} ${m[0]}`),
    );
    expect(hits).toEqual([]);
  });

  it("tokens.css defines every core token", () => {
    const css = readFileSync(join(ROOT, "tokens.css"), "utf8");
    for (const t of [
      "--slate-0: #090b0c",
      "--slate-1: #0c0f10",
      "--slate-2: #15191b",
      "--slate-3: #262b2e",
      "--text: #e8e8ed",
      "--body: #cacbd3",
      "--muted: #9699a5",
      "--pink: #ffadc6",
      "--pink-deep: #c3698a",
      "--pink-soft: rgba(255, 173, 198, 0.12)",
      "--green: #5fbf8f",
      "--amber: #d9a441",
      "--red: #e06c75",
    ]) {
      expect(css).toContain(t);
    }
  });
});
