// Helpers for saved sessions: zone maths, formatting and CSV (mirrors kittytooth-core's history
// module so the in-browser mock exports the same file the app does).

import type { Config, Retention, Session, SessionSummary, SourceKind } from "./types";
import { SOURCE_INFO } from "./format";

type Zones = Config["zones"];

export function maxHrOf(z: Zones): number {
  return z.age && z.age >= 1 && z.age <= 120 ? 220 - z.age : Math.min(250, Math.max(100, z.manual_max_hr));
}

/** Zone 0 (below zone 1) to 5, same rule as the engine. */
export function zoneOf(bpm: number, z: Zones): number {
  const pct = (Math.round(bpm) * 100) / maxHrOf(z);
  return z.thresholds.filter((t) => pct >= t).length;
}

export const RETENTION_LABELS: Record<Retention, string> = {
  days30: "30 days",
  days90: "90 days",
  days365: "1 year",
  forever: "forever",
};

/** Unix ms as an ISO 8601 UTC timestamp with milliseconds. */
export function isoUtc(ms: number): string {
  return new Date(ms).toISOString();
}

/** CSV with one row per stored sample, identical to the app's export. */
export function sessionCsv(s: Session): string {
  const lines = ["timestamp,elapsed_s,bpm,raw,rr_ms,zone,alert"];
  const start = s.summary.start_ms;
  let ai = 0;
  for (const p of s.samples) {
    const fired: string[] = [];
    while (ai < s.alerts.length && s.alerts[ai].t_ms <= p.t_ms) {
      fired.push(`${s.alerts[ai].kind} ${s.alerts[ai].bpm}`);
      ai++;
    }
    lines.push(
      [
        isoUtc(p.t_ms),
        ((p.t_ms - start) / 1000).toFixed(1),
        p.bpm.toFixed(1),
        p.raw,
        p.rr_ms.join(" "),
        zoneOf(p.bpm, s.zones),
        fired.join(" "),
      ].join(","),
    );
  }
  return `${lines.join("\n")}\n`;
}

export function csvFileName(startMs: number): string {
  const iso = isoUtc(startMs);
  return `kittytooth-${iso.slice(0, 10)}-${iso.slice(11, 13)}${iso.slice(14, 16)}.csv`;
}

/** "1 h 12 min", "42 min", "50 s". */
export function formatSpan(ms: number): string {
  const s = Math.max(0, Math.round(ms / 1000));
  if (s < 60) return `${s} s`;
  const m = Math.round(s / 60);
  if (m < 60) return `${m} min`;
  const h = Math.floor(m / 60);
  const r = m % 60;
  return r ? `${h} h ${r} min` : `${h} h`;
}

/** "Today", "Yesterday", or "Tue 24 Nov" (with the year when it isn't this year). */
export function formatDay(ms: number, now = Date.now()): string {
  const d = new Date(ms);
  const today = new Date(now);
  const startOf = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((startOf(today) - startOf(d)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  const opts: Intl.DateTimeFormatOptions = { weekday: "short", day: "numeric", month: "short" };
  if (d.getFullYear() !== today.getFullYear()) opts.year = "numeric";
  return d.toLocaleDateString(undefined, opts);
}

/** "19:40" in the user's locale. */
export function formatTime(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function formatBytes(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1024 * 1024) return `${Math.round(n / 1024)} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}

/** "Polar H10" or the source name when no device label was saved. */
export function sourceLabel(s: Pick<SessionSummary, "device" | "source">): string {
  return s.device ?? SOURCE_INFO[s.source as SourceKind].name;
}
