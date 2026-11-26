import type {
  BodySensorLocation,
  Capabilities,
  ConnState,
  Metric,
  Preset,
  SensorContact,
  Snapshot,
  SourceKind,
} from "./types";

export const SOURCE_ORDER: SourceKind[] = ["ble", "ant", "pulsoid", "hyperate", "osc", "demo"];

export const SOURCE_INFO: Record<SourceKind, { name: string; tag: string; help: string }> = {
  ble: { name: "Bluetooth sensor", tag: "ble", help: "A chest strap, armband or watch in heart rate broadcast mode." },
  ant: {
    name: "ANT+ sensor",
    tag: "ant",
    help: "A strap or watch broadcasting over ANT+, through a USB ANT+ stick. Gives Garmin watches HRV.",
  },
  pulsoid: { name: "Pulsoid", tag: "pulsoid", help: "Heart rate from the Pulsoid app, through Pulsoid's real-time API." },
  hyperate: { name: "HypeRate", tag: "hyperate", help: "Heart rate from the HypeRate app, by its device ID." },
  osc: { name: "OSC input", tag: "osc", help: "Relay a heart rate another app sends to KittyTooth over OSC." },
  demo: { name: "Demo strap", tag: "demo", help: "Simulated readings for trying KittyTooth without hardware." },
};

/** Readings the source can't provide, in display order ("hrv", "battery", "skin contact"). */
export function missingFeatures(c: Capabilities): string[] {
  const out: string[] = [];
  if (!c.rr) out.push("hrv");
  if (!c.battery) out.push("battery");
  if (!c.contact) out.push("skin contact");
  return out;
}

export const ZONE_NAMES = ["Rest", "Warm-up", "Fat burn", "Aerobic", "Threshold", "Max"] as const;

export function formatBpm(v: number | null | undefined): string {
  return v == null || !Number.isFinite(v) ? "--" : String(Math.round(v));
}

export function formatMs(v: number | null | undefined): string {
  return v == null || !Number.isFinite(v) ? "--" : `${Math.round(v)}`;
}

export function zoneName(z: number): string {
  return ZONE_NAMES[Math.max(0, Math.min(5, Math.round(z)))];
}


/** "just now", "5 s ago", "3 min ago", "2 h ago". */
export function timeAgo(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((now - ms) / 1000));
  if (s < 2) return "just now";
  if (s < 60) return `${s} s ago`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ago`;
  return `${Math.floor(m / 60)} h ago`;
}

/** 0-4 signal bars from RSSI in dBm. */
export function rssiBars(rssi: number | null | undefined): number {
  if (rssi == null) return 0;
  if (rssi >= -60) return 4;
  if (rssi >= -70) return 3;
  if (rssi >= -80) return 2;
  if (rssi >= -90) return 1;
  return 0;
}

export function rssiLabel(rssi: number | null | undefined): string {
  return ["No signal", "Weak", "Fair", "Good", "Excellent"][rssiBars(rssi)];
}

export function formatClock(ms: number): string {
  const d = new Date(ms);
  const p = (n: number, w = 2) => String(n).padStart(w, "0");
  return `${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}.${p(d.getMilliseconds(), 3)}`;
}

export function formatDuration(totalSec: number): string {
  const s = Math.max(0, Math.round(totalSec));
  const h = Math.floor(s / 3600);
  const m = Math.floor((s % 3600) / 60);
  const r = s % 60;
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${String(r).padStart(2, "0")}` : `${m}:${String(r).padStart(2, "0")}`;
}

export interface StatusView {
  label: string;
  tone: "ok" | "warn" | "busy" | "off" | "error";
}

export function connectionStatus(s: Pick<Snapshot, "state" | "data_valid" | "contact" | "test_sweep" | "attempt" | "error">): StatusView {
  if (s.test_sweep) return { label: "Test sweep", tone: "busy" };
  const map: Record<ConnState, StatusView> = {
    idle: { label: "Not connected", tone: "off" },
    connecting: { label: "Connecting", tone: "busy" },
    connected: { label: "Connected", tone: "ok" },
    no_data: { label: s.contact === "not_detected" ? "No skin contact" : "Waiting for data", tone: "warn" },
    reconnecting: { label: s.error ? "Retrying" : "Reconnecting", tone: s.error ? "error" : "busy" },
  };
  return map[s.state];
}

export function contactLabel(c: SensorContact): string | null {
  if (c === "detected") return "Good skin contact";
  if (c === "not_detected") return "No skin contact: wet the electrodes";
  return null;
}

export function locationLabel(l: BodySensorLocation | null): string | null {
  if (!l) return null;
  return { other: "Other", chest: "Chest", wrist: "Wrist", finger: "Finger", hand: "Hand", ear_lobe: "Ear lobe", foot: "Foot", unknown: null }[l] ?? null;
}

export const PRESET_LABELS: Record<Preset, string> = {
  kitty_tooth: "KittyTooth",
  hr_to_vrchat: "HRtoVRChat compatible",
  vrcosc: "VRCOSC Heartrate compatible",
  custom: "Custom",
};

export const METRIC_LABELS: Record<Metric, string> = {
  hr: "Heart rate (bpm)",
  hr_raw: "Heart rate, raw",
  normalized: "Normalized 0..1",
  normalized_signed: "Normalized -1..1",
  percent: "Percent of max HR 0..1",
  ones: "Ones digit",
  tens: "Tens digit",
  hundreds: "Hundreds digit",
  connected: "Connected",
  beat_toggle: "Beat (toggle)",
  beat_pulse: "Beat (100 ms pulse)",
  zone: "Zone 0-5",
  hrv_ms: "HRV (ms)",
  hrv_normalized: "HRV 0..1",
  battery: "Battery 0..1",
  session_min: "Session min",
  session_avg: "Session average",
  session_max: "Session max",
};

/** Mirrors osc::normalize_address in Rust. Returns the full address or null if invalid. */
export function normalizeOscAddress(input: string): string | null {
  const s = input.trim();
  if (!s) return null;
  const full = s.startsWith("/") ? s : `/avatar/parameters/${s}`;
  if (full.length > 256 || full.endsWith("/") || full.includes("//") || /[\s#*,?[\]{}]/.test(full)) return null;
  // eslint-disable-next-line no-control-regex
  if (/[\u0000-\u001f]/.test(full)) return null;
  return full;
}

export function isValidHost(h: string): boolean {
  const s = h.trim();
  if (!s || s.length > 253) return false;
  if (/^\d{1,3}(\.\d{1,3}){3}$/.test(s)) return s.split(".").every((p) => Number(p) <= 255);
  if (s.includes(":")) return /^[0-9a-fA-F:]+$/.test(s); // IPv6
  return /^[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?(\.[a-zA-Z0-9]([a-zA-Z0-9-]*[a-zA-Z0-9])?)*$/.test(s);
}

export function isValidPort(p: number): boolean {
  return Number.isInteger(p) && p >= 1 && p <= 65535;
}

export function renderChatbox(template: string, s: Snapshot, phase = true): string {
  const dash = (v: number | null | undefined, d = 0) => (v == null ? "--" : v.toFixed(d));
  return template
    .replaceAll("{hr}", formatBpm(s.bpm))
    .replaceAll("{heart}", phase ? "❤️" : "\u{1f497}")
    .replaceAll("{min}", dash(s.stats.min))
    .replaceAll("{avg}", dash(s.stats.avg))
    .replaceAll("{max}", dash(s.stats.max))
    .replaceAll("{hrv}", dash(s.hrv_ms))
    .replaceAll("{zone}", String(s.zone))
    .replaceAll("{battery}", dash(s.battery))
    .slice(0, 144);
}
