// Helpers for the in-app updater UI. The Rust side decides whether a version is newer and
// whether this copy can install it; these only shape what the UI shows.

export interface NoteBlock {
  kind: "heading" | "item" | "text";
  text: string;
}

/** Strips the bits of GitHub Markdown that read badly as plain text. */
function plain(s: string): string {
  return s
    .replace(/\[([^\]]+)\]\([^)]+\)/g, "$1") // [label](url) -> label
    .replace(/(\*\*|__)(.+?)\1/g, "$2") // bold
    .replace(/`([^`]+)`/g, "$1") // code
    .replace(/<!--.*?-->/g, "")
    .trim();
}

/**
 * Turns a release body (GitHub's generated notes are Markdown) into simple blocks the UI can
 * render as text. Nothing is rendered as HTML, so a release body can't inject markup.
 */
export function notesToBlocks(md: string | null | undefined, maxBlocks = 40): NoteBlock[] {
  if (!md) return [];
  const out: NoteBlock[] = [];
  for (const raw of md.replace(/\r\n?/g, "\n").split("\n")) {
    const line = raw.trim();
    if (!line) continue;
    const h = /^#{1,6}\s+(.*)$/.exec(line);
    const li = /^[-*+]\s+(.*)$/.exec(line);
    const text = plain(h ? h[1] : li ? li[1] : line);
    if (!text) continue;
    out.push({ kind: h ? "heading" : li ? "item" : "text", text });
    if (out.length >= maxBlocks) break;
  }
  return out;
}

/** "4.2 MB" style sizes for the download progress. */
export function formatBytes(n: number): string {
  if (!Number.isFinite(n) || n < 0) return "--";
  if (n < 1024) return `${Math.round(n)} B`;
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(0)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** Download progress as a whole percentage, or null when the size is unknown. */
export function progressPercent(downloaded: number, total: number | null): number | null {
  if (!total || total <= 0) return null;
  return Math.max(0, Math.min(100, Math.round((downloaded / total) * 100)));
}
