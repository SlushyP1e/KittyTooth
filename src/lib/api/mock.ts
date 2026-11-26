// In-browser simulated backend. Used when the UI runs outside Tauri: `npm run dev`, Vitest,
// Playwright smoke tests and README screenshots. It mimics the Rust engine's behaviour:
// scanning, connecting, a realistic heart rate with RR-timed beats, session stats, HRV,
// OSC log entries and the test sweep.

import type { Backend, Unlisten } from "./backend";
import { defaultConfig, emptySnapshot } from "../defaults";
import { csvFileName, sessionCsv } from "../sessions";
import { mockSessions } from "./mock-sessions";
import type {
  BackendEvents,
  Config,
  ConfigResult,
  DiscoveredDevice,
  HistoryPoint,
  InitialState,
  OscLogEntry,
  Snapshot,
  AppInfo,
  Capabilities,
  SecretKind,
  SecretStatus,
  Session,
  SourceKind,
  UpdateInfo,
  UserError,
} from "../types";

type AnyListener = (p: unknown) => void;

const DEVICES: Omit<DiscoveredDevice, "last_seen_ms">[] = [
  { id: "demo:strap", name: "KittyTooth Demo Strap", rssi: -52, has_hr_service: true, demo: true },
  { id: "demo:watch", name: "KittyTooth Demo Watch", rssi: -71, has_hr_service: true, demo: true },
  { id: "mock:h10", name: "Polar H10 7E21A3", rssi: -63, has_hr_service: true, demo: false },
  { id: "mock:coospo", name: "COOSPO H808S", rssi: -79, has_hr_service: true, demo: false },
];
const OTHER_DEVICES: Omit<DiscoveredDevice, "last_seen_ms">[] = [
  { id: "mock:mouse", name: "MX Master 3S", rssi: -58, has_hr_service: false, demo: false },
  { id: "mock:buds", name: "Galaxy Buds2 Pro", rssi: -84, has_hr_service: false, demo: false },
];

const CAPS: Record<SourceKind, Capabilities> = {
  ble: { rr: true, battery: true, contact: true },
  demo: { rr: true, battery: true, contact: true },
  ant: { rr: true, battery: true, contact: false },
  pulsoid: { rr: false, battery: true, contact: false },
  hyperate: { rr: false, battery: false, contact: false },
  osc: { rr: false, battery: false, contact: false },
};

function maskSecret(v: string): string {
  return v.length <= 4 ? "••••" : `••••${v.slice(-4)}`;
}

/** Demo target heart rate at loop time t (s); same profile as the Rust demo strap. */
export function demoTargetHr(t: number): number {
  const loop = 360;
  const x = ((t % loop) + loop) % loop;
  const ease = (v: number) => v * v * (3 - 2 * v);
  if (x < 60) return 64;
  if (x < 180) return 64 + (158 - 64) * ease((x - 60) / 120);
  if (x < 240) return 158 + 6 * Math.sin((x - 180) / 9);
  if (x < 330) return 70 + (158 - 70) * Math.exp(-(x - 240) / 25);
  return 66;
}

/** Deterministic PRNG so screenshots and tests are stable. */
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

export interface MockOptions {
  /** "demo" (default): auto-connected demo strap; "idle": nothing remembered; "error": radio off. */
  scenario?: "demo" | "idle" | "error";
  /** Virtual loop time at start, seconds. */
  startAt?: number;
  /** Real milliseconds per sample. */
  sampleMs?: number;
  /** Prefilled history in seconds. */
  prefill?: number;
  /** Fake update on offer: "available" (default), "portable", "none" or "error". */
  update?: "available" | "portable" | "none" | "error";
}

/** The pretend release the demo UI offers. */
export const MOCK_UPDATE_VERSION = "1.3.0";
const MOCK_UPDATE_NOTES = `## What's new
* Check for updates from Settings, with release notes right in the app
* Smoother heart animation at high BPM

## Fixes
* The tray icon no longer flickers when the sensor reconnects

**Full Changelog**: https://github.com/SlushyP1e/KittyTooth/compare/v1.2.0...v1.3.0`;

export interface MockBackend extends Backend {
  dispose(): void;
  /** Test hook: current internal snapshot. */
  peek(): Snapshot;
}

export function createMockBackend(opts: MockOptions = {}): MockBackend {
  const params = typeof location !== "undefined" ? new URLSearchParams(location.search) : new URLSearchParams();
  const scenario = opts.scenario ?? (params.get("mock") as MockOptions["scenario"]) ?? "demo";
  const sampleMs = opts.sampleMs ?? 1000;
  const updateMode = opts.update ?? (params.get("update") as MockOptions["update"]) ?? "available";
  let updateInstalled = false;
  const prefill = opts.prefill ?? 300;
  const rand = mulberry32(1234);
  const listeners = new Map<keyof BackendEvents, Set<AnyListener>>();
  const timers = new Set<ReturnType<typeof setTimeout>>();
  let config: Config = defaultConfig();
  let snap: Snapshot = emptySnapshot();
  const history: HistoryPoint[] = [];
  const log: OscLogEntry[] = [];
  let logSeq = 1;
  let vt = opts.startAt ?? 168; // virtual loop seconds (mid exercise ramp)
  let beatToggle = false;
  let sweepUntil = 0;
  let scanTimer: ReturnType<typeof setInterval> | null = null;
  const devices = new Map<string, DiscoveredDevice>();
  const rrWindow: { t: number; rr: number }[] = [];
  let sampleTimer: ReturnType<typeof setTimeout> | null = null;
  let beatTimer: ReturnType<typeof setTimeout> | null = null;
  let disposed = false;
  let sessions: Session[] | null = null;
  const savedSessions = () => (sessions ??= mockSessions());
  const findSession = (id: string): Session => {
    const s = savedSessions().find((x) => x.summary.id === id);
    if (!s)
      throw {
        code: "invalid_input",
        title: "Session not found",
        message: "That session no longer exists.",
        fix: "Pick another session from the list.",
      } satisfies UserError;
    return s;
  };

  const info: AppInfo = {
    version: "1.0.0",
    platform: "browser (demo mode)",
    overlay_url: "http://127.0.0.1:9123/overlay",
    overlay_error: null,
    log_dir: "%APPDATA%\\app.kittytooth.desktop\\logs",
    config_path: "%APPDATA%\\app.kittytooth.desktop\\config.json",
    history_dir: "%LOCALAPPDATA%\\app.kittytooth.desktop\\sessions",
    notice: null,
    secrets: [],
    hyperate_builtin_key: true,
  };
  const secrets = new Map<SecretKind, string>();
  function secretStatuses(): SecretStatus[] {
    return (["pulsoid_token", "hyperate_key"] as SecretKind[]).map((kind) => {
      const v = secrets.get(kind);
      return { kind, present: !!v, masked: v ? maskSecret(v) : null };
    });
  }
  info.secrets = secretStatuses();

  function emit<K extends keyof BackendEvents>(event: K, payload: BackendEvents[K]) {
    listeners.get(event)?.forEach((cb) => cb(structuredClone(payload)));
  }
  function later(ms: number, fn: () => void) {
    const t = setTimeout(() => {
      timers.delete(t);
      if (!disposed) fn();
    }, ms);
    timers.add(t);
    return t;
  }
  function publish() {
    const fb = config.source.fallback;
    snap.source = config.source.active;
    snap.live_source = snap.data_valid || snap.test_sweep ? snap.source : null;
    snap.sources = [];
    if (snap.state !== "idle" || snap.error) {
      snap.sources.push({
        role: "primary",
        kind: snap.source,
        label: snap.device?.name ?? null,
        state: snap.state,
        error: snap.error,
        last_packet_ms: snap.last_packet_ms,
        live: true,
      });
      if (fb.enabled && fb.backup !== snap.source) {
        snap.sources.push({
          role: "backup",
          kind: fb.backup,
          label: fb.backup === "demo" ? "KittyTooth Demo Strap" : null,
          state: "connected",
          error: null,
          last_packet_ms: snap.last_packet_ms,
          live: false,
        });
      }
    }
    emit("snapshot", snap);
  }
  function pushLog(address: string, value: string, kind: string, test = false) {
    log.push({ seq: logSeq++, t_ms: Date.now(), target: "VRChat", address, value, kind, test });
    if (snap.targets[0]) snap.targets[0].stats.sent++;
    if (log.length > 50) log.splice(0, log.length - 50);
  }
  function rmssd(): number | null {
    const now = Date.now();
    while (rrWindow.length && now - rrWindow[0].t > 30_000) rrWindow.shift();
    if (rrWindow.length < 5) return null;
    let s = 0;
    for (let i = 1; i < rrWindow.length; i++) s += (rrWindow[i].rr - rrWindow[i - 1].rr) ** 2;
    return Math.sqrt(s / (rrWindow.length - 1));
  }
  function maxHr() {
    const a = config.zones.age;
    return a && a > 0 && a <= 120 ? 220 - a : config.zones.manual_max_hr;
  }
  function zone(bpm: number) {
    const pct = (bpm * 100) / maxHr();
    return config.zones.thresholds.filter((t) => pct >= t).length;
  }
  function hrAt(t: number) {
    return demoTargetHr(t) + (rand() - 0.5) * 2.4;
  }

  // Prefill history so the graph has something to show immediately.
  function doPrefill() {
    const now = Date.now();
    for (let i = prefill; i > 0; i--) {
      const t = vt - i;
      const bpm = Math.round(hrAt(t));
      history.push({ t_ms: now - i * 1000, bpm, raw: bpm });
      snap.stats.samples++;
      snap.stats.min = Math.min(snap.stats.min ?? bpm, bpm);
      snap.stats.max = Math.max(snap.stats.max ?? bpm, bpm);
      snap.stats.avg = ((snap.stats.avg ?? bpm) * (snap.stats.samples - 1) + bpm) / snap.stats.samples;
    }
  }

  function scheduleBeat() {
    if (beatTimer) clearTimeout(beatTimer);
    if (!(snap.data_valid || snap.test_sweep) || snap.bpm == null) return;
    const bpm = snap.test_sweep ? sweepBpm() : snap.bpm;
    const breath = Math.sin((Date.now() / 1000) * ((2 * Math.PI) / 4.8));
    const rsa = Math.max(0, 1 - (bpm - 60) / 110) * 0.06;
    const interval = (60_000 / bpm) * (1 + rsa * breath + (rand() - 0.5) * 0.02);
    beatTimer = later(interval, () => {
      beatToggle = !beatToggle;
      rrWindow.push({ t: Date.now(), rr: interval });
      emit("beat", { interval_ms: Math.round(interval), test: snap.test_sweep });
      pushLog("/avatar/parameters/KittyTooth/Beat", String(beatToggle), "bool", snap.test_sweep);
      pushLog("/avatar/parameters/KittyTooth/BeatPulse", "true", "bool", snap.test_sweep);
      scheduleBeat();
    });
  }

  function sweepBpm() {
    const left = Math.max(0, sweepUntil - Date.now());
    const x = 1 - left / 16_000;
    return 60 + 120 * (1 - Math.abs(2 * x - 1));
  }

  function sample() {
    if (snap.state !== "connected" && snap.state !== "no_data") return;
    vt += 1;
    const raw = Math.round(hrAt(vt));
    const smooth =
      config.processing.smoothing === "off"
        ? raw
        : Math.round((snap.bpm ?? raw) * (config.processing.smoothing === "light" ? 0.5 : 0.75) +
            raw * (config.processing.smoothing === "light" ? 0.5 : 0.25));
    const point = { t_ms: Date.now(), bpm: smooth, raw };
    history.push(point);
    const st = snap.stats;
    st.samples++;
    st.min = Math.min(st.min ?? raw, raw);
    st.max = Math.max(st.max ?? raw, raw);
    st.avg = ((st.avg ?? raw) * (st.samples - 1) + raw) / st.samples;
    const wasValid = snap.data_valid;
    snap = {
      ...snap,
      state: "connected",
      bpm: smooth,
      raw_bpm: raw,
      data_valid: true,
      contact: !snap.capabilities.contact || snap.device?.id === "demo:watch" ? "not_supported" : "detected",
      zone: zone(smooth),
      max_hr: maxHr(),
      hrv_ms: snap.capabilities.rr ? rmssd() : null,
      last_packet_ms: Date.now(),
      packets: snap.packets + 1,
      energy_kj: snap.device?.id === "demo:strap" ? Math.round(vt * 1.7) : null,
      latency: {
        last_us: 380 + Math.round(rand() * 300),
        avg_us: 512,
        max_us: 1840,
        samples: snap.latency.samples + 1,
      },
    };
    if (!snap.test_sweep) {
      const kt = "/avatar/parameters/KittyTooth/";
      pushLog(kt + "HR", String(smooth), "int");
      pushLog(kt + "Normalized", ((smooth - 40) / 160).toFixed(3), "float");
      pushLog(kt + "Ones", String(smooth % 10), "int");
    }
    emit("sample", point);
    publish();
    if (!wasValid) scheduleBeat();
    sampleTimer = later(sampleMs, sample);
  }

  function stopStreaming() {
    if (sampleTimer) clearTimeout(sampleTimer);
    if (beatTimer) clearTimeout(beatTimer);
  }

  function connectTo(id: string, name: string, kind: SourceKind = id.startsWith("demo:") ? "demo" : "ble") {
    stopStreaming();
    const bluetooth = kind === "ble" || kind === "demo";
    config.source.active = kind;
    if (bluetooth) config.device.last_device = { id, name };
    emit("config", config);
    snap = {
      ...snap,
      state: "connecting",
      target: bluetooth ? { id, name } : null,
      device: null,
      data_valid: false,
      error: null,
      attempt: 0,
      capabilities: CAPS[kind],
    };
    publish();
    later(700, () => {
      snap = {
        ...snap,
        state: "no_data",
        device: {
          id,
          name,
          location: bluetooth ? (id === "demo:watch" ? "wrist" : "chest") : null,
          demo: id.startsWith("demo:"),
        },
        battery: !CAPS[kind].battery || id === "demo:watch" ? null : kind === "pulsoid" ? 64 : 87,
      };
      publish();
      sampleTimer = later(300, sample);
    });
  }

  function failSource(kind: SourceKind, error: UserError | null) {
    stopStreaming();
    config.source.active = kind;
    emit("config", config);
    snap = {
      ...snap,
      state: "idle",
      device: null,
      target: null,
      data_valid: false,
      battery: null,
      error,
      capabilities: CAPS[kind],
    };
    publish();
  }

  function startSource(kind: SourceKind) {
    const last = config.device.last_device;
    switch (kind) {
      case "ble":
        if (last && !last.id.startsWith("demo:")) connectTo(last.id, last.name, "ble");
        else failSource("ble", null);
        return;
      case "demo":
        if (last?.id.startsWith("demo:")) connectTo(last.id, last.name, "demo");
        else connectTo("demo:strap", "KittyTooth Demo Strap", "demo");
        return;
      case "pulsoid": {
        const tok = secrets.get("pulsoid_token");
        if (!tok)
          return failSource("pulsoid", {
            code: "missing_credential",
            title: "Pulsoid token needed",
            message: "KittyTooth needs your Pulsoid access token to read your heart rate.",
            fix: "Press Get token on the Source page, create a token on Pulsoid, and paste it into the token field.",
          });
        if (tok === "bad")
          return failSource("pulsoid", {
            code: "bad_token",
            title: "Pulsoid token rejected",
            message: "Pulsoid says the access token is invalid, revoked or expired.",
            fix: "Create a new token with Get token on the Source page and paste it in. KittyTooth stops retrying until you do.",
          });
        return connectTo("pulsoid", "Pulsoid", "pulsoid");
      }
      case "hyperate": {
        const id = config.source.hyperate.device_id.trim();
        if (!/^[A-Za-z0-9_-]{2,32}$/.test(id))
          return failSource("hyperate", {
            code: "invalid_device_id",
            title: "HypeRate device ID not accepted",
            message: id ? `HypeRate didn't accept the device ID "${id}".` : "No HypeRate device ID has been entered.",
            fix: "Copy the ID exactly as the HypeRate app shows it (letters and digits only) into the Device ID field.",
          });
        return connectTo(`hyperate:${id}`, `HypeRate ${id}`, "hyperate");
      }
      case "ant":
        if (scenario === "error")
          return failSource("ant", {
            code: "ant_stick_missing",
            title: "No ANT+ stick found",
            message: "KittyTooth can't find a USB ANT+ stick on this PC.",
            fix: "Plug in an ANT+ USB stick (Garmin USB2 / USB-m or a compatible clone). KittyTooth picks it up automatically; a USB extension cable keeps it close to you.",
          });
        return connectTo("ant:12345", "ANT+ HRM 12345", "ant");
      case "osc": {
        const port = config.source.osc_input.port;
        return connectTo(`osc:${port}`, `OSC input :${port}`, "osc");
      }
    }
  }

  function init() {
    snap.adapter = scenario === "error" ? "off" : "ready";
    snap.oscquery = { active: true, mdns: true, http_port: 51234, osc_port: 51235, error: null };
    snap.vrchat = { service: "VRChat-Client-4F2A1C._oscjson._tcp.local.", ip: "127.0.0.1", port: 9000, local: true };
    snap.targets = [
      {
        id: "vrchat",
        name: "VRChat",
        enabled: true,
        dest: "127.0.0.1:9000",
        via_oscquery: false,
        stats: { sent: 0, coalesced: 0, errors: 0, last_error: null },
      },
    ];
    snap.max_hr = maxHr();
    if (scenario === "error") {
      snap.error = {
        code: "radio_off",
        title: "Bluetooth is turned off",
        message: "The Bluetooth radio is switched off, so sensors can't be reached.",
        fix: "Turn Bluetooth on in Windows Settings > Bluetooth & devices (or the Action Center). KittyTooth will reconnect on its own.",
      };
      snap.state = "reconnecting";
      snap.attempt = 3;
      snap.retry_at_ms = Date.now() + 4000;
      config.device.last_device = { id: "mock:h10", name: "Polar H10 7E21A3" };
      snap.target = config.device.last_device;
    } else if (scenario === "demo") {
      doPrefill();
      config.device.last_device = { id: "demo:strap", name: "KittyTooth Demo Strap" };
      config.source.active = "demo";
      snap.target = config.device.last_device;
      snap.state = "connected";
      snap.device = { id: "demo:strap", name: "KittyTooth Demo Strap", location: "chest", demo: true };
      snap.battery = 87;
      snap.data_valid = true;
      const last = history[history.length - 1];
      snap.bpm = last?.bpm ?? 72;
      snap.raw_bpm = snap.bpm;
      snap.zone = zone(snap.bpm);
      // Seed HRV window.
      const now = Date.now();
      for (let i = 0; i < 24; i++) rrWindow.push({ t: now - (24 - i) * 450, rr: 430 + (rand() - 0.5) * 18 });
      snap.hrv_ms = rmssd();
      sampleTimer = later(sampleMs, sample);
      later(50, scheduleBeat);
    }
  }
  init();

  const api: MockBackend = {
    isMock: true,
    dispose() {
      disposed = true;
      timers.forEach((t) => clearTimeout(t));
      timers.clear();
      if (scanTimer) clearInterval(scanTimer);
    },
    peek: () => structuredClone(snap),
    async getState(): Promise<InitialState> {
      return structuredClone({ snapshot: snap, config, devices: [...devices.values()], info });
    },
    async setConfig(c: Config): Promise<ConfigResult> {
      const next = structuredClone(c);
      next.device.stale_timeout_secs = Math.min(60, Math.max(2, next.device.stale_timeout_secs));
      next.osc.max_rate = Math.min(20, Math.max(1, next.osc.max_rate));
      next.osc.chatbox.interval_secs = Math.max(2, next.osc.chatbox.interval_secs);
      const sourceChanged = JSON.stringify(config.source) !== JSON.stringify(next.source);
      config = next;
      snap.max_hr = maxHr();
      if (sourceChanged && (snap.state !== "idle" || snap.error)) startSource(config.source.active);
      snap.targets = config.osc.targets.map((t) => ({
        id: t.id,
        name: t.name,
        enabled: t.enabled,
        dest: t.enabled ? `${t.host}:${t.port}` : null,
        via_oscquery: false,
        stats: { sent: 0, coalesced: 0, errors: 0, last_error: null },
      }));
      info.overlay_url = config.overlay.enabled ? `http://127.0.0.1:${config.overlay.port}/overlay` : null;
      publish();
      return structuredClone({ config, info });
    },
    async startScan() {
      snap.scanning = true;
      publish();
      const tick = () => {
        const list = [...DEVICES, ...(config.device.show_all_devices ? OTHER_DEVICES : [])].filter(
          (d) => config.device.demo_device || !d.demo,
        );
        for (const d of list) {
          devices.set(d.id, { ...d, rssi: (d.rssi ?? -70) + Math.round((rand() - 0.5) * 8), last_seen_ms: Date.now() });
        }
        for (const id of [...devices.keys()]) if (!list.some((d) => d.id === id)) devices.delete(id);
        emit("devices", [...devices.values()].sort((a, b) => Number(b.has_hr_service) - Number(a.has_hr_service) || (b.rssi ?? -999) - (a.rssi ?? -999)));
      };
      if (scanTimer) clearInterval(scanTimer);
      later(150, tick);
      scanTimer = setInterval(tick, 1000);
    },
    async stopScan() {
      if (scanTimer) clearInterval(scanTimer);
      scanTimer = null;
      snap.scanning = false;
      publish();
    },
    async connect(id, name) {
      connectTo(id, name);
    },
    async selectSource(kind) {
      startSource(kind);
    },
    async setSecret(kind, value) {
      const v = value.trim();
      if (!v)
        throw {
          code: "invalid_input",
          title: "Invalid value",
          message: "That doesn't look like a valid token or key.",
          fix: "Correct the highlighted value and try again.",
        };
      secrets.set(kind, v);
      info.secrets = secretStatuses();
      if (snap.state !== "idle" || snap.error) startSource(config.source.active);
      return secretStatuses();
    },
    async clearSecret(kind) {
      secrets.delete(kind);
      info.secrets = secretStatuses();
      return secretStatuses();
    },
    async openLink() {},
    async disconnect() {
      stopStreaming();
      snap = { ...snap, state: "idle", device: null, target: null, data_valid: false, battery: null, error: null };
      pushLog("/avatar/parameters/KittyTooth/Connected", "false", "bool");
      publish();
    },
    async forget() {
      await api.disconnect();
      config.device.last_device = null;
      emit("config", config);
    },
    async testSweep(on) {
      snap.test_sweep = on;
      if (on) {
        sweepUntil = Date.now() + 16_000;
        const step = () => {
          if (!snap.test_sweep) return;
          if (Date.now() >= sweepUntil) {
            snap.test_sweep = false;
            publish();
            scheduleBeat();
            return;
          }
          pushLog("/avatar/parameters/KittyTooth/HR", String(Math.round(sweepBpm())), "int", true);
          later(250, step);
        };
        step();
        scheduleBeat();
      }
      publish();
    },
    async testAlert() {
      const threshold = config.alerts.high_bpm;
      const bpm = threshold + 12;
      snap.alert = { kind: "high", bpm, threshold, since_ms: Date.now(), test: true };
      emit("alert", { kind: "high", bpm, threshold, repeat: false, test: true });
      if (config.alerts.chatbox) pushLog("/chatbox/input", config.alerts.high_text.replace("{hr}", String(bpm)), "string", true);
      publish();
      later(8000, () => {
        if (snap.alert?.test) {
          snap.alert = null;
          publish();
        }
      });
    },
    async resetSession() {
      history.length = 0;
      snap.stats = { min: null, max: null, avg: null, samples: 0 };
      publish();
    },
    async getHistory(sinceMs) {
      return history.filter((p) => p.t_ms > sinceMs);
    },
    async getOscLog() {
      return structuredClone(log);
    },
    async listSessions() {
      return structuredClone(savedSessions().map((s) => s.summary));
    },
    async getSession(id) {
      return structuredClone(findSession(id));
    },
    async deleteSession(id) {
      findSession(id);
      sessions = savedSessions().filter((s) => s.summary.id !== id);
      emit("sessions", null);
      return structuredClone(sessions.map((s) => s.summary));
    },
    async exportSession(id) {
      const s = findSession(id);
      const name = csvFileName(s.summary.start_ms);
      if (typeof document !== "undefined" && typeof URL.createObjectURL === "function") {
        const blob = new Blob([sessionCsv(s)], { type: "text/csv" });
        const a = document.createElement("a");
        a.href = URL.createObjectURL(blob);
        a.download = name;
        a.click();
        URL.revokeObjectURL(a.href);
      }
      return name;
    },
    async exportConfig() {
      if (typeof document !== "undefined" && typeof URL.createObjectURL === "function") {
        const blob = new Blob([JSON.stringify(config, null, 2)], { type: "application/json" });
        const a = document.createElement("a");
        a.href = URL.createObjectURL(blob);
        a.download = "kittytooth-settings.json";
        a.click();
        URL.revokeObjectURL(a.href);
      }
      return "kittytooth-settings.json";
    },
    async importConfig() {
      return null;
    },
    async resetConfig() {
      await api.forget();
      return api.setConfig(defaultConfig());
    },
    async copyDiagnostics() {
      const text = `KittyTooth diagnostics (demo mode)\n${JSON.stringify({ snapshot: snap, config }, null, 2)}`;
      try {
        await navigator.clipboard?.writeText(text);
      } catch {
        /* clipboard may be unavailable in tests */
      }
      return text;
    },
    async openMini() {
      if (typeof window !== "undefined") window.open("#/mini", "kittytooth-mini", "width=232,height=112");
    },
    async closeMini() {
      if (typeof window !== "undefined" && location.hash === "#/mini") window.close();
    },
    async showMain() {},
    async startDrag() {},
    async checkUpdate(): Promise<UpdateInfo | null> {
      await new Promise<void>((r) => later(400, r));
      if (updateMode === "error") {
        throw {
          code: "update_check_failed",
          title: "Couldn't check for updates",
          message: "GitHub didn't answer, or the release has no update information yet.",
          fix: "Check your internet connection and try again later.",
        } satisfies UserError;
      }
      if (updateMode === "none" || updateInstalled) return null;
      const portable = updateMode === "portable";
      return {
        current: info.version,
        version: MOCK_UPDATE_VERSION,
        notes: MOCK_UPDATE_NOTES,
        date: "2026-11-20T12:00:00Z",
        install_kind: portable ? "portable" : "installer",
        can_install: !portable,
        release_url: `https://github.com/SlushyP1e/KittyTooth/releases/tag/v${MOCK_UPDATE_VERSION}`,
      };
    },
    async installUpdate() {
      if (updateMode === "portable") throw new Error("portable copies can't install");
      const total = 9_400_000;
      for (let d = 0; d < total; ) {
        await new Promise<void>((r) => later(120, r));
        d = Math.min(total, d + 1_900_000);
        emit("update_progress", { downloaded: d, total });
      }
      // Pretend the app restarted on the new version.
      updateInstalled = true;
      info.version = MOCK_UPDATE_VERSION;
    },
    async openRelease(version: string) {
      if (typeof window !== "undefined") window.open(`https://github.com/SlushyP1e/KittyTooth/releases/tag/v${version}`, "_blank", "noopener");
    },
    async on<K extends keyof BackendEvents>(event: K, cb: (p: BackendEvents[K]) => void): Promise<Unlisten> {
      let set = listeners.get(event);
      if (!set) listeners.set(event, (set = new Set()));
      const fn = cb as AnyListener;
      set.add(fn);
      return () => void set.delete(fn);
    },
  };
  return api;
}
