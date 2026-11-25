import { describe, expect, it } from "vitest";
import { applyTheme, readableOn, resolveTheme, withAlpha } from "./theme";

describe("theme", () => {
  it("applies theme and preset accents via data attributes", () => {
    const el = document.createElement("div");
    applyTheme("light", "peach", el);
    expect(el.dataset.theme).toBe("light");
    expect(el.dataset.accent).toBe("peach");
    expect(el.style.getPropertyValue("--pink")).toBe("");
    applyTheme("dark", "not-a-color", el);
    expect(el.dataset.theme).toBe("dark");
    expect(el.dataset.accent).toBe("pink");
  });

  it("derives the accent trio from a custom color", () => {
    const el = document.createElement("div");
    const custom = "#" + "33aa77";
    applyTheme("dark", custom, el);
    expect(el.dataset.accent).toBe("custom");
    expect(el.style.getPropertyValue("--pink")).toBe(custom);
    expect(el.style.getPropertyValue("--pink-deep")).toContain("color-mix");
    expect(el.style.getPropertyValue("--pink-soft")).toContain("transparent");
    applyTheme("dark", "pink", el);
    expect(el.style.getPropertyValue("--pink")).toBe("");
  });

  it("picks readable text on custom accents", () => {
    expect(readableOn("#" + "ffffff")).toBe("black");
    expect(readableOn("#" + "ffadc6")).toBe("black");
    expect(readableOn("#" + "1a1033")).toBe("white");
    expect(readableOn("nope")).toBe("white");
  });

  it("converts computed colors to rgba", () => {
    expect(withAlpha("#" + "ff0080", 0.1)).toBe("rgba(255, 0, 128, 0.1)");
    expect(withAlpha("rgb(1, 2, 3)", 0.5)).toBe("rgba(1, 2, 3, 0.5)");
  });

  it("resolves system theme from the media query", () => {
    expect(resolveTheme("dark")).toBe("dark");
    expect(resolveTheme("system")).toBe("dark"); // jsdom matchMedia stub: no light preference
  });
});
