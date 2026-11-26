// Fake past sessions for the in-browser mock (dev, tests, screenshots). Deterministic: the same
// seed always gives the same heart rate traces, placed relative to "now".

import { defaultConfig } from "../defaults";
import { zoneOf } from "../sessions";
import type { HrvPoint, Session, SessionAlert, SessionSample, SessionSummary, SourceKind } from "../types";

function mulberry32(seed: number) {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

interface Plan {
  daysAgo: number;
  hour: number;
  minute: number;
  minutes: number;
  source: SourceKind;
  device: string | null;
  rr: boolean;
  seed: number;
  /** Target heart rate at fraction x (0..1) of the session. */
  shape: (x: number) => number;
  alerts: { at: number; kind: "high" | "low"; threshold: number }[];
}

const bump = (x: number, c: number, w: number) => Math.exp(-(((x - c) / w) ** 2));

const PLANS: Plan[] = [
  {
    // An evening in VRChat with a dance world in the middle.
    daysAgo: 1,
    hour: 20,
    minute: 12,
    minutes: 108,
    source: "ble",
    device: "Polar H10 7E21A3",
    rr: true,
    seed: 11,
    shape: (x) => 78 + 8 * Math.sin(x * 9) + 70 * bump(x, 0.55, 0.06) + 20 * bump(x, 0.3, 0.05),
    alerts: [{ at: 0.55, kind: "high", threshold: 140 }],
  },
  {
    // A long, calm social session through Pulsoid (no beat intervals).
    daysAgo: 3,
    hour: 19,
    minute: 34,
    minutes: 155,
    source: "pulsoid",
    device: null,
    rr: false,
    seed: 23,
    shape: (x) => 72 + 6 * Math.sin(x * 14) + 10 * bump(x, 0.7, 0.1),
    alerts: [],
  },
  {
    // A morning workout on an ANT+ strap.
    daysAgo: 6,
    hour: 7,
    minute: 15,
    minutes: 42,
    source: "ant",
    device: "HRM-Pro 41022",
    rr: true,
    seed: 37,
    shape: (x) => (x < 0.12 ? 70 + 400 * x : x < 0.85 ? 118 + 40 * Math.sin((x - 0.12) * 11) ** 2 : 160 - (x - 0.85) * 500),
    alerts: [
      { at: 0.3, kind: "high", threshold: 140 },
      { at: 0.58, kind: "high", threshold: 140 },
    ],
  },
  {
    // Falling asleep in VR.
    daysAgo: 12,
    hour: 23,
    minute: 2,
    minutes: 65,
    source: "ble",
    device: "Polar H10 7E21A3",
    rr: true,
    seed: 41,
    shape: (x) => 68 - 22 * x + 3 * Math.sin(x * 30),
    alerts: [{ at: 0.9, kind: "low", threshold: 50 }],
  },
];

function hrvSeries(samples: SessionSample[]): HrvPoint[] {
  const beats: [number, number][] = [];
  for (const s of samples) s.rr_ms.forEach((rr) => rr >= 300 && rr <= 2000 && beats.push([s.t_ms, rr]));
  const out: HrvPoint[] = [];
  if (!beats.length) return out;
  for (let t = beats[0][0] + 60_000; t <= beats[beats.length - 1][0]; t += 15_000) {
    const win = beats.filter((b) => b[0] > t - 60_000 && b[0] <= t);
    const diffs: number[] = [];
    for (let i = 1; i < win.length; i++) {
      const d = win[i][1] - win[i - 1][1];
      if (Math.abs(d) <= 0.2 * win[i - 1][1]) diffs.push(d);
    }
    if (diffs.length >= 10) {
      const v = Math.sqrt(diffs.reduce((a, d) => a + d * d, 0) / diffs.length);
      out.push({ t_ms: t, rmssd_ms: Math.round(v * 10) / 10 });
    }
  }
  return out;
}

function build(p: Plan, now: number): Session {
  const rand = mulberry32(p.seed);
  const zones = defaultConfig().zones;
  const d = new Date(now);
  const start = new Date(d.getFullYear(), d.getMonth(), d.getDate() - p.daysAgo, p.hour, p.minute, 7).getTime();
  const n = p.minutes * 60;
  const samples: SessionSample[] = [];
  let bpm = p.shape(0);
  let drift = 0;
  for (let i = 0; i < n; i++) {
    // A short dropout (strap slipped) in the longer sessions.
    if (n > 3000 && i > n * 0.42 && i < n * 0.42 + 40) continue;
    drift = drift * 0.98 + (rand() - 0.5) * 0.9;
    const target = p.shape(i / n) + drift;
    bpm += (target - bpm) * 0.15;
    const raw = Math.round(bpm + (rand() - 0.5) * 3);
    const rr: number[] = [];
    if (p.rr) {
      // Beat intervals with breathing-rate variation, more of it when the heart rate is low.
      const base = 60_000 / raw;
      const sway = ((110 - Math.min(110, raw)) * 0.9 + 6) * 2.5;
      rr.push(Math.round(base + Math.sin(i * 1.3) * sway * 0.5 + (rand() - 0.5) * sway * 0.3));
    }
    samples.push({ t_ms: start + i * 1000, bpm: Math.round(bpm * 10) / 10, raw, rr_ms: rr });
  }
  const alerts: SessionAlert[] = p.alerts.map((a) => {
    const s = samples[Math.min(samples.length - 1, Math.floor(a.at * samples.length))];
    const bpm = a.kind === "high" ? Math.max(s.raw, a.threshold + 3) : Math.min(s.raw, a.threshold - 2);
    return { t_ms: s.t_ms, kind: a.kind, bpm, threshold: a.threshold };
  });
  const zone_secs: Session["zone_secs"] = [0, 0, 0, 0, 0, 0];
  samples.forEach((s, i) => {
    const next = samples[i + 1];
    const span = Math.min(5000, next ? next.t_ms - s.t_ms : 1000);
    zone_secs[zoneOf(s.bpm, zones)] += span / 1000;
  });
  const raws = samples.map((s) => s.raw);
  const summary: SessionSummary = {
    id: String(start),
    start_ms: start,
    end_ms: samples[samples.length - 1].t_ms,
    source: p.source,
    device: p.device,
    min_bpm: Math.min(...raws),
    avg_bpm: Math.round(raws.reduce((a, b) => a + b, 0) / raws.length),
    max_bpm: Math.max(...raws),
    samples: samples.length,
    alerts: alerts.length,
    has_rr: p.rr,
    active: false,
    bytes: samples.length * (p.rr ? 19 : 15),
  };
  return {
    summary,
    zones,
    max_hr: 190,
    samples,
    alerts,
    zone_secs: zone_secs.map(Math.round) as Session["zone_secs"],
    hrv: hrvSeries(samples),
  };
}

/** Four plausible past sessions, newest first. */
export function mockSessions(now = Date.now()): Session[] {
  return PLANS.map((p) => build(p, now));
}
