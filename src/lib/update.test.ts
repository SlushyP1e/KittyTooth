import { describe, expect, it } from "vitest";
import { formatBytes, notesToBlocks, progressPercent } from "./update";

describe("release notes", () => {
  it("turns GitHub Markdown into plain blocks", () => {
    const blocks = notesToBlocks(
      "## What's Changed\r\n* Add updater by @someone in [#8](https://example.test/8)\n\n- **Bold** `code`\n<!-- hidden -->\n**Full Changelog**: v1...v2",
    );
    expect(blocks).toEqual([
      { kind: "heading", text: "What's Changed" },
      { kind: "item", text: "Add updater by @someone in #8" },
      { kind: "item", text: "Bold code" },
      { kind: "text", text: "Full Changelog: v1...v2" },
    ]);
  });

  it("handles empty notes and caps long ones", () => {
    expect(notesToBlocks(null)).toEqual([]);
    expect(notesToBlocks("   \n\n")).toEqual([]);
    expect(notesToBlocks(Array.from({ length: 100 }, (_, i) => `* ${i}`).join("\n"), 5)).toHaveLength(5);
  });

  it("never passes markup through as HTML", () => {
    const [b] = notesToBlocks("<img src=x onerror=alert(1)>");
    expect(b.kind).toBe("text");
    // Rendered with {text} interpolation, so this stays inert text.
    expect(b.text).toContain("<img");
  });
});

describe("download progress", () => {
  it("formats sizes", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2 KB");
    expect(formatBytes(9_400_000)).toBe("9.0 MB");
    expect(formatBytes(-1)).toBe("--");
  });

  it("computes a clamped percentage, or null when the size is unknown", () => {
    expect(progressPercent(50, 200)).toBe(25);
    expect(progressPercent(300, 200)).toBe(100);
    expect(progressPercent(10, null)).toBeNull();
    expect(progressPercent(10, 0)).toBeNull();
  });
});
