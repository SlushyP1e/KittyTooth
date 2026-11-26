// TypeScript mirror of the Rust types serialized by the backend (serde snake_case).

export interface UserError {
  code: string;
  title: string;
  message: string;
  fix: string;
}

export type ConnState = "idle" | "connecting" | "connected" | "no_data" | "reconnecting";
export type AdapterState = "ready" | "off" | "missing" | "unknown";
export type SensorContact = "not_supported" | "not_detected" | "detected";
export type BodySensorLocation = "other" | "chest" | "wrist" | "finger" | "hand" | "ear_lobe" | "foot" | "unknown";

export type SourceKind = "ble" | "ant" | "pulsoid" | "hyperate" | "osc" | "demo";
export type SecretKind = "pulsoid_token" | "hyperate_key";

/** What a source can report; missing items are "not available from this source". */
export interface Capabilities {
  rr: boolean;
  battery: boolean;
  contact: boolean;
}

export interface SlotStatus {
  role: "primary" | "backup";
  kind: SourceKind;
  label: string | null;
  state: ConnState;
  error: UserError | null;
  last_packet_ms: number | null;
  live: boolean;
}

export interface SecretStatus {
  kind: SecretKind;
  present: boolean;
  masked: string | null;
}

export interface DeviceInfo {
  id: string;
  name: string;
  location: BodySensorLocation | null;
  demo: boolean;
}

export interface RememberedDevice {
  id: string;
  name: string;
}

export interface SessionStats {
  min: number | null;
  max: number | null;
  avg: number | null;
  samples: number;
}

export interface TargetStats {
  sent: number;
  coalesced: number;
  errors: number;
  last_error: string | null;
}

export interface TargetStatus {
  id: string;
  name: string;
  enabled: boolean;
  dest: string | null;
  via_oscquery: boolean;
  stats: TargetStats;
}

export interface VrchatEndpoint {
  service: string;
  ip: string;
  port: number;
  local: boolean;
}

export interface OscQueryStatus {
  active: boolean;
  mdns: boolean;
  http_port: number | null;
  osc_port: number | null;
  error: string | null;
}

export interface LatencyStats {
  last_us: number;
  avg_us: number;
  max_us: number;
  samples: number;
}

export interface Snapshot {
  state: ConnState;
  attempt: number;
  retry_at_ms: number | null;
  device: DeviceInfo | null;
  target: RememberedDevice | null;
  bpm: number | null;
  raw_bpm: number | null;
  data_valid: boolean;
  contact: SensorContact;
  stats: SessionStats;
  hrv_ms: number | null;
  zone: number;
  max_hr: number;
  energy_kj: number | null;
  battery: number | null;
  last_packet_ms: number | null;
  adapter: AdapterState;
  error: UserError | null;
  scan_error: UserError | null;
  scanning: boolean;
  test_sweep: boolean;
  targets: TargetStatus[];
  vrchat: VrchatEndpoint | null;
  oscquery: OscQueryStatus;
  latency: LatencyStats;
  rejected_spikes: number;
  rejected_out_of_range: number;
  packets: number;
  source: SourceKind;
  live_source: SourceKind | null;
  fallback_active: boolean;
  capabilities: Capabilities;
  sources: SlotStatus[];
  /** High or low heart rate alert showing right now. */
  alert: AlertStatus | null;
}

export type AlertKind = "high" | "low";

export interface AlertStatus {
  kind: AlertKind;
  bpm: number;
  threshold: number;
  since_ms: number;
  test: boolean;
}

export interface AlertConfig {
  enabled: boolean;
  high_bpm: number;
  low_bpm: number;
  sustain_secs: number;
  repeat_secs: number;
  notify: boolean;
  sound: boolean;
  chatbox: boolean;
  high_text: string;
  low_text: string;
}

export interface AlertEvent {
  kind: AlertKind;
  bpm: number;
  threshold: number;
  repeat: boolean;
  test: boolean;
}

export interface DiscoveredDevice {
  id: string;
  name: string;
  rssi: number | null;
  has_hr_service: boolean;
  last_seen_ms: number;
  demo: boolean;
}

export interface HistoryPoint {
  t_ms: number;
  bpm: number;
  raw: number;
}

export interface OscLogEntry {
  seq: number;
  t_ms: number;
  target: string;
  address: string;
  value: string;
  kind: string;
  test: boolean;
}

export type Smoothing = "off" | "light" | "medium";
export type Preset = "kitty_tooth" | "hr_to_vrchat" | "vrcosc" | "custom";
export type ValueKind = "int" | "float" | "bool";
export type Metric =
  | "hr"
  | "hr_raw"
  | "normalized"
  | "normalized_signed"
  | "percent"
  | "ones"
  | "tens"
  | "hundreds"
  | "connected"
  | "beat_toggle"
  | "beat_pulse"
  | "zone"
  | "hrv_ms"
  | "hrv_normalized"
  | "battery"
  | "session_min"
  | "session_avg"
  | "session_max";
export type DisconnectBehavior = "hold_last" | "send_zero";
export type Motion = "on" | "reduced" | "system";
export type Theme = "dark" | "light" | "system";
export type GraphRange = "60s" | "5m" | "session";
export type VrAttach = "left_wrist" | "right_wrist" | "hud";
export type VrSize = "small" | "medium" | "large";
export type VrHand = "left" | "right" | "both";

export interface SteamVrConfig {
  enabled: boolean;
  /** Show the heart rate overlay in the headset. */
  overlay: boolean;
  attach: VrAttach;
  size: VrSize;
  /** Percent, 20..100. */
  opacity: number;
  /** Buzz the controllers when an alert fires or reminds. */
  haptics: boolean;
  haptics_hand: VrHand;
}

export type SteamVrState = "off" | "not_installed" | "not_running" | "connected" | "error";

export interface SteamVrStatus {
  state: SteamVrState;
  version: string | null;
  message: string | null;
}

export interface CustomMapping {
  address: string;
  metric: Metric;
  kind: ValueKind;
}

export interface OscTarget {
  id: string;
  name: string;
  host: string;
  port: number;
  enabled: boolean;
  preset: Preset;
  custom: CustomMapping[];
  use_oscquery: boolean;
  chatbox: boolean;
  range: [number, number] | null;
}

export interface ChatboxConfig {
  enabled: boolean;
  template: string;
  interval_secs: number;
  silent: boolean;
}

export interface Config {
  schema_version: number;
  device: {
    last_device: RememberedDevice | null;
    auto_connect: boolean;
    show_all_devices: boolean;
    stale_timeout_secs: number;
    demo_device: boolean;
  };
  source: {
    active: SourceKind;
    hyperate: { device_id: string };
    osc_input: { port: number; address: string; lan: boolean };
    fallback: { enabled: boolean; backup: SourceKind; after_secs: number };
  };
  processing: { smoothing: Smoothing };
  zones: { age: number | null; manual_max_hr: number; thresholds: [number, number, number, number, number] };
  alerts: AlertConfig;
  osc: {
    targets: OscTarget[];
    keepalive_ms: number;
    max_rate: number;
    on_disconnect: DisconnectBehavior;
    oscquery: boolean;
    chatbox: ChatboxConfig;
  };
  ui: {
    theme: Theme;
    accent: string;
    start_minimized: boolean;
    close_to_tray: boolean;
    autostart: boolean;
    mini_always_on_top: boolean;
    graph_range: GraphRange;
    motion: Motion;
  };
  overlay: { enabled: boolean; port: number };
  steamvr: SteamVrConfig;
  debug: { log_latency: boolean };
}

export interface AppInfo {
  version: string;
  platform: string;
  overlay_url: string | null;
  overlay_error: UserError | null;
  log_dir: string;
  config_path: string;
  notice: UserError | null;
  secrets: SecretStatus[];
  hyperate_builtin_key: boolean;
}

export interface InitialState {
  snapshot: Snapshot;
  config: Config;
  devices: DiscoveredDevice[];
  info: AppInfo;
  steamvr: SteamVrStatus;
}

export interface ConfigResult {
  config: Config;
  info: AppInfo;
}

export interface BeatEvent {
  interval_ms: number;
  test: boolean;
}

export interface BackendEvents {
  snapshot: Snapshot;
  beat: BeatEvent;
  sample: HistoryPoint;
  devices: DiscoveredDevice[];
  config: Config;
  alert: AlertEvent;
  steamvr: SteamVrStatus;
}
