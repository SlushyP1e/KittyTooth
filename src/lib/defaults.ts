import type { Config, OscTarget, Snapshot } from "./types";

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
    schema_version: 2,
    device: {
      last_device: null,
      auto_connect: true,
      show_all_devices: false,
      stale_timeout_secs: 5,
      demo_device: true,
    },
    processing: { smoothing: "off" },
    zones: { age: null, manual_max_hr: 190, thresholds: [50, 60, 70, 80, 90] },
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
      accent: "#ff6fa8",
      start_minimized: false,
      close_to_tray: true,
      autostart: false,
      mini_always_on_top: true,
      graph_range: "60s",
    },
    overlay: { enabled: true, port: 9123 },
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
    rejected_out_of_range: 0,
    packets: 0,
  };
}

export const PRESET_DEFAULT_RANGE: Record<OscTarget["preset"], [number, number]> = {
  kitty_tooth: [40, 200],
  custom: [40, 200],
  hr_to_vrchat: [0, 255],
  vrcosc: [0, 240],
};
