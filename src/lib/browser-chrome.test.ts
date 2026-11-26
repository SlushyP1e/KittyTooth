import { afterEach, describe, expect, it } from "vitest";
import { installAppChrome, isBrowserShortcut } from "./browser-chrome";

let uninstall: (() => void) | null = null;
afterEach(() => {
  uninstall?.();
  uninstall = null;
  document.body.innerHTML = "";
});

function rightClick(el: Element): boolean {
  const ev = new MouseEvent("contextmenu", { bubbles: true, cancelable: true });
  el.dispatchEvent(ev);
  return ev.defaultPrevented;
}

describe("app chrome", () => {
  it("blocks the browser menu except in text fields", () => {
    uninstall = installAppChrome();
    document.body.innerHTML = `<div id="d">x</div><input id="t" type="text" /><input id="c" type="checkbox" /><textarea id="a"></textarea>`;
    expect(rightClick(document.getElementById("d")!)).toBe(true);
    expect(rightClick(document.getElementById("c")!)).toBe(true);
    expect(rightClick(document.getElementById("t")!)).toBe(false);
    expect(rightClick(document.getElementById("a")!)).toBe(false);
  });

  it("recognises print and reload shortcuts", () => {
    const k = (key: string, mods: Partial<KeyboardEvent> = {}) =>
      isBrowserShortcut({ key, ctrlKey: false, metaKey: false, shiftKey: false, ...mods });
    expect(k("p", { ctrlKey: true })).toBe(true);
    expect(k("F5")).toBe(true);
    expect(k("I", { ctrlKey: true, shiftKey: true })).toBe(true);
    expect(k("c", { ctrlKey: true })).toBe(false);
    expect(k("v", { ctrlKey: true })).toBe(false);
    expect(k("a")).toBe(false);
  });
});
