import { describe, expect, it } from "vitest";
import { applyTheme, resolveTheme } from "./theme";

describe("theme", () => {
  it("applies theme and accent with readable text color", () => {
    const el = document.createElement("div");
    applyTheme("light", "#1a1033", el);
    expect(el.dataset.theme).toBe("light");
    expect(el.style.getPropertyValue("--accent")).toBe("#1a1033");
    expect(el.style.getPropertyValue("--accent-text")).toBe("#ffffff");
    applyTheme("dark", "not-a-color", el);
    expect(el.dataset.theme).toBe("dark");
    expect(el.style.getPropertyValue("--accent")).toBe("#ff6fa8");
  });

  it("resolves system theme from the media query", () => {
    expect(resolveTheme("dark")).toBe("dark");
    expect(resolveTheme("system")).toBe("dark"); // jsdom matchMedia stub: no light preference
  });
});
