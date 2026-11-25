# KittyTooth — Build Plan

KittyTooth reads heart rate from Bluetooth LE sensors (Heart Rate Service `0x180D`) and
forwards it to VRChat and other OSC apps. Primary target: Windows 11.

## Architecture

```
crates/kittytooth-core     pure Rust, no Tauri: everything testable on Linux CI
  hrm.rs                   0x2A37 Heart Rate Measurement parser (+ encoder for the mock)
  processing.rs            range check, spike rejection, EMA smoothing, session stats,
                           RMSSD (30 s window), zones, digit splitting
  beat.rs                  beat scheduler (RR playback, BPM fallback)
  config.rs                versioned JSON config + migrations
  error.rs                 typed errors with human message + fix suggestion
  sensor/                  HeartRateBackend trait, mock backend, btleplug backend (feature "ble")
  osc/                     presets, encoder, per-target sender with rate limiting, chatbox,
                           OSCQuery (mDNS advertise + VRChat discovery)
  engine.rs                connection manager (auto-connect, backoff reconnect, stale
                           detection), processing pipeline, OSC output. Independent of UI.
  overlay.rs               tiny local HTTP server: /overlay (OBS browser source), /events (SSE)
  tray_icon.rs             renders the live BPM into a tray icon bitmap
src-tauri                  Tauri 2 shell: commands, events (throttled), tray, mini window,
                           logging, autostart, single instance
src/                       Svelte 5 + TypeScript UI; `api/` abstracts Tauri vs. in-browser mock
e2e/                       Playwright smoke tests + README screenshots (mock backend)
```

The OSC path lives entirely in the engine task: BLE notification → parser → processor →
OSC encoder → UDP, in one task with no UI round-trip.

## Milestones

1. **Scaffold** — workspace, frontend skeleton, PLAN/DECISIONS.
2. **Core logic** — parser, processing, config + migration, errors; full unit tests.
3. **Engine + OSC** — sensor trait, mock sensor, reconnect/stale, beats, presets, rate limiter,
   chatbox, OSCQuery, overlay server; UDP integration tests for every preset.
4. **BLE + Tauri shell** — btleplug backend with Windows error mapping, commands/events, tray
   with BPM icon, mini window, rotating logs, diagnostics, autostart.
5. **Frontend** — main/devices/OSC/settings screens, canvas graph, theming, a11y; Vitest,
   eslint, svelte-check, Playwright smoke + screenshots.
6. **Ship** — CI (lint/test/build on push, Windows + Linux), release workflow on tags,
   README/PARAMETERS/DEVICES/CHECKLIST, self review, green CI.

After every milestone: `cargo fmt --check`, `cargo clippy -D warnings`, `cargo test`,
`npm run check`, `npm run lint`, `npm test`.
