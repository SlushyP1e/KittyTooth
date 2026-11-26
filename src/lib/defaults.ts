import type { AlertConfig, Config, OscTarget, Snapshot, SteamVrConfig } from "./types";

export function defaultAlerts(): AlertConfig {
  return {
    enabled: false,
    high_bpm: 150,
    low_bpm: 45,
    sustain_secs: 10,
    repeat_secs: 120,
    notify: true,
    sound: true,
    chatbox: false,
    high_text: "\u26a0\ufe0f High heart rate: {hr} bpm",
    low_text: "\u26a0\ufe0f Low heart rate: {hr} bpm",
  };
}

export function defaultSteamVr(): SteamVrConfig {
  return {
    enabled: false,
    overlay: true,
    attach: "left_wrist",
    size: "medium",
    opacity: 90,
    haptics: true,
    haptics_hand: "both",
  };
}

// Mirrors the Rust defaults in crates/kittytooth-core/src/config.rs.
export function defaultTarget(id = "vrchat", name = "VRChat"): OscTarget {
  return {
    id,
    name,
    host: "127.0.0.1",
    port: 9000,
    enabled: true,
    preset: "kitty_tooth",
    custom: [],
    use_oscquery: true,
    chatbox: true,
    range: null,
  };
}

export function defaultConfig(): Config {
  return {
    schema_version: 3,
    device: {
      last_device: null,
      auto_connect: true,
      show_all_devices: false,
      stale_timeout_secs: 5,
      demo_device: true,
    },
    source: {
      active: "ble",
      hyperate: { device_id: "" },
      osc_input: { port: 9010, address: "/avatar/parameters/HR", lan: false },
      fallback: { enabled: false, backup: "demo", after_secs: 5 },
    },
    processing: { smoothing: "off" },
    zones: { age: null, manual_max_hr: 190, thresholds: [50, 60, 70, 80, 90] },
    alerts: defaultAlerts(),
    osc: {
      targets: [defaultTarget()],
      keepalive_ms: 1000,
      max_rate: 20,
      on_disconnect: "hold_last",
      oscquery: true,
      chatbox: { enabled: false, template: "{hr} bpm {heart}", interval_secs: 2, silent: true },
    },
    ui: {
      theme: "dark",
      accent: "pink",
      start_minimized: false,
      close_to_tray: true,
      autostart: false,
      mini_always_on_top: true,
      graph_range: "60s",
      motion: "on",
    },
    overlay: { enabled: true, port: 9123 },
    steamvr: defaultSteamVr(),
    debug: { log_latency: false },
  };
}

export function emptySnapshot(): Snapshot {
  return {
    state: "idle",
    attempt: 0,
    retry_at_ms: null,
    device: null,
    target: null,
    bpm: null,
    raw_bpm: null,
    data_valid: false,
    contact: "not_supported",
    stats: { min: null, max: null, avg: null, samples: 0 },
    hrv_ms: null,
    zone: 0,
    max_hr: 190,
    energy_kj: null,
    battery: null,
    last_packet_ms: null,
    adapter: "unknown",
    error: null,
    scan_error: null,
    scanning: false,
    test_sweep: false,
    targets: [],
    vrchat: null,
    oscquery: { active: false, mdns: false, http_port: null, osc_port: null, error: null },
    latency: { last_us: 0, avg_us: 0, max_us: 0, samples: 0 },
    rejected_spikes: 0,
    source: "ble",
    live_source: null,
    fallback_active: false,
    capabilities: { rr: true, battery: true, contact: true },
    sources: [],
    rejected_out_of_range: 0,
    packets: 0,
    alert: null,
  };
}

export const PRESET_DEFAULT_RANGE: Record<OscTarget["preset"], [number, number]> = {
  kitty_tooth: [40, 200],
  custom: [40, 200],
  hr_to_vrchat: [0, 255],
  vrcosc: [0, 240],
};
