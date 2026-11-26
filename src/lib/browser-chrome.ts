// Hides the web-browser leftovers of the WebView so KittyTooth feels like a desktop app:
// no right-click menu (Print, Inspect, Reload...) except in text fields, where copy and paste
// are useful, and no print or reload shortcuts.

function isEditable(t: EventTarget | null): boolean {
  if (!(t instanceof Element)) return false;
  const el = t.closest("input, textarea, [contenteditable=''], [contenteditable='true']");
  if (!el) return false;
  if (el instanceof HTMLInputElement) {
    return !["checkbox", "radio", "range", "color", "button", "submit", "reset", "file"].includes(el.type);
  }
  return true;
}

/** True for shortcuts that would print, reload, or open browser pages. */
export function isBrowserShortcut(e: Pick<KeyboardEvent, "key" | "ctrlKey" | "metaKey" | "shiftKey">): boolean {
  const mod = e.ctrlKey || e.metaKey;
  const k = e.key.toLowerCase();
  if (k === "f5" || k === "f7" || k === "browserrefresh") return true;
  if (mod && (k === "p" || k === "r" || k === "u" || k === "j" || k === "g" || k === "f")) return true;
  if (mod && e.shiftKey && (k === "i" || k === "c" || k === "delete")) return true;
  return false;
}

export function installAppChrome(doc: Document = document): () => void {
  const onMenu = (e: MouseEvent) => {
    if (!isEditable(e.target)) e.preventDefault();
  };
  const onKey = (e: KeyboardEvent) => {
    if (isBrowserShortcut(e)) e.preventDefault();
  };
  doc.addEventListener("contextmenu", onMenu);
  doc.addEventListener("keydown", onKey, true);
  return () => {
    doc.removeEventListener("contextmenu", onMenu);
    doc.removeEventListener("keydown", onKey, true);
  };
}
