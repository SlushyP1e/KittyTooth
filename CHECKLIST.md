# KittyTooth requirements checklist

Each line of the spec, where it is implemented, and how it is verified.
`core` = `crates/kittytooth-core/src`, `it` = `crates/kittytooth-core/tests/pipeline.rs`,
`e2e` = `e2e/smoke.spec.ts`. Items marked **(HW)** also need real-hardware checks on Windows
(listed at the end).

## Environment
- [x] BLE behind a trait: `core/sensor/mod.rs` (`HeartRateBackend`), btleplug impl `core/sensor/ble.rs`
- [x] Realistic mock sensor (resting, ramp, dropouts, RR, contact loss): `core/sensor/mock.rs`; used by `it` and the in-browser mock `src/lib/api/mock.ts`
- [x] OSC proven against a local UDP listener: `it` (all tests)
- [x] Windows builds on windows-latest: `.github/workflows/ci.yml` (`build-windows`), `release.yml`

## Stack
- [x] Tauri 2, btleplug, rosc, tokio, serde, Svelte 5 + TS + Vite, hand-built components (DECISIONS.md "Stack deviations": none)
- [x] JSON config in app data dir with schema version + migration: `core/config.rs` (`migrates_v1`, `store_migrates_on_disk_with_backup`, `store_recovers_from_corrupt_file`)
- [x] NSIS installer + portable exe via Actions; Linux AppImage: `ci.yml`, `release.yml`

## Bluetooth
- [x] Scan for 0x180D, show name, RSSI bars, last seen, "show all devices": `core/sensor/ble.rs`, `src/lib/views/DevicesView.svelte`; e2e "devices screen scans, lists and connects"
- [x] Subscribe 0x2A37 and parse flags, u8/u16, contact, energy, all RR (1/1024 s): `core/hrm.rs`; tests `spec_example_*`, `every_flag_combination_roundtrips`, `energy_and_multiple_rr`, `contact_status_values`, `rr_conversion_roundtrip`, `truncated_packets_error` **(HW)**
- [x] Battery 0x2A19 and Body Sensor Location 0x2A38 if present: `core/sensor/ble.rs`, `core/hrm.rs` (`battery_and_location`) **(HW)**
- [x] Remember last device, auto-connect on launch: `core/engine.rs`, `config.device`; Devices screen toggle
- [x] Reconnect backoff 1/2/4/8/10 s forever until user disconnects: `core/engine.rs` (`backoff_sequence`); it `reconnect_backoff_after_failures`, `link_loss_reconnects_without_user_action`
- [x] Stale detection, configurable (default 5 s): `core/engine.rs`; it `stale_data_marks_disconnected_and_reconnects`; Settings "Stale timeout"
- [x] Windows quirks with human messages (radio off, adapter missing, device busy): `core/error.rs`, `core/sensor/ble.rs`; e2e "errors are shown in plain language with a fix" **(HW)**
- [x] Device profiles and quirks: `docs/DEVICES.md` **(HW)**

## Data processing
- [x] EMA smoothing off/light/medium, raw always available: `core/processing.rs` (`ema_math`, `smoothing_levels`); `Snapshot.raw_bpm`
- [x] Reject <25 / >250 and single-sample spikes: `processing.rs` (`rejects_out_of_range`, `single_sample_spike_rejected`, `confirmed_jump_accepted`); it `spikes_out_of_range_and_contact_loss_are_filtered`
- [x] Current, session min/avg/max, RMSSD 30 s rolling, zones from max HR (220-age or manual): `processing.rs` (`session_stats`, `rmssd_*`, `rolling_rmssd_*`, `zones`, `zone_sanitize`)
- [x] Beat on each RR, else from BPM: `core/beat.rs` (`rr_replay_follows_real_intervals`, `bpm_fallback_timing`, `no_burst_after_pause`)

## OSC output
- [x] Default 127.0.0.1:9000, multiple targets each with host/port/enabled/preset: `core/config.rs` (`defaults_are_vrchat`), `core/osc/sender.rs`; it `multiple_targets_receive_their_own_preset`; OSC screen
- [x] OSCQuery advertise + VRChat discovery, manual fallback: `core/osc/oscquery.rs`; it `avatar_change_via_oscquery_resends_everything` **(HW)**
- [x] On change + 1 s keepalive (configurable), cap 20 msg/s/target: `osc/sender.rs` (`only_changes_are_sent`, `keepalive_resends_unchanged`, `limiter_never_exceeds_cap_in_any_window`); it `rate_limit_never_exceeded_under_flood`, `keepalive_resends_every_second`
- [x] Preset 1 KittyTooth native (all 13 params): `osc/presets.rs` (`kittytooth_addresses_and_types`); it `kittytooth_preset_exact_values`
- [x] Preset 2 HRtoVRChat: `presets.rs` (`hrtovrchat_addresses_and_types`); it `hrtovrchat_preset_exact_values`
- [x] Preset 3 VRCOSC Heartrate: `presets.rs` (`vrcosc_addresses_and_types`); it `vrcosc_preset_exact_values`
- [x] Preset 4 Custom (any metric, int/float/bool): `presets.rs` (`custom_mapping_types`, `every_metric_maps`); it `custom_preset_exact_values`
- [x] Names verified against upstream source with commits cited: DECISIONS.md "Upstream parameter names"
- [x] Chatbox template, 2 s minimum, silent option: `osc/chatbox.rs`, `sender.rs` (`chatbox_respects_min_interval`, `chatbox_with_sound`); it `chatbox_template_and_min_interval`
- [x] Disconnect: Connected=false once, hold last or send 0: it `disconnect_sends_connected_false_once_and_holds`, `disconnect_send_zero_mode`

## UI and design
- [x] Minimal, cute not childish, cat-ear heart motif; dark default, light theme, accent picker: `src/tokens.css`, `src/lib/theme.ts`, `Heart.svelte`, `Logo.svelte`; e2e "settings: theme, accent, diagnostics"; screenshots in `docs/screenshots`
- [x] Main screen: big BPM, beat-synced heart, status, battery, canvas graph 60 s/5 min/session, zone band, min/avg/max, HRV: `LiveView.svelte`, `HrGraph.svelte`, `ZoneBand.svelte`; e2e "live screen shows BPM, beat-synced heart and a painted graph"
- [x] Devices screen: scan list, connect/forget, signal: `DevicesView.svelte`; e2e devices test
- [x] OSC screen: targets, presets, live log of last 50 (pausable), test sweep: `OscView.svelte`, `TargetCard.svelte`; it `test_sweep_works_without_sensor`; e2e "OSC screen"
- [x] Settings: smoothing, stale timeout, zones, autostart, start minimized, theme, export/import, reset: `SettingsView.svelte`
- [x] Tray icon with live BPM, connect/disconnect, quit: `src-tauri/src/tray.rs`, `core/tray_icon.rs` **(HW)**
- [x] Always-on-top mini window: `src-tauri/src/windows.rs`, `MiniView.svelte`; e2e "mini window shows BPM"
- [x] OBS source at http://127.0.0.1:<port>/overlay, transparent: `core/overlay.rs`, `core/overlay.html`; it `overlay_serves_html_and_state`
- [x] Keyboard accessible, visible focus, reduced motion, contrast, tabular numerals: `src/app.css`; e2e "keyboard navigation...", "respects reduced motion", "BPM text never shifts layout"; `theme.test.ts`
- [x] Plain-language errors with fix, no stack traces: `core/error.rs` (`UserError`), `ErrorBanner.svelte`; e2e errors test

## Performance
- [x] Idle CPU <1%, RAM <120 MB: graph loop only runs while live and visible; UI throttled (DECISIONS.md "Frontend") **(HW)**
- [x] BLE-to-OSC latency <10 ms measured and logged in debug: `core/engine.rs` (`LatencyStats`), shown in Settings > Diagnostics **(HW)**
- [x] Tauri events throttled to display rate; OSC never depends on UI: `src-tauri/src/lib.rs` (forwarder); engine is a standalone tokio task

## Quality bar
- [x] cargo fmt, clippy -D warnings: `ci.yml` `rust` job
- [x] Unit tests: parser (every flag combo), RMSSD, smoothing, spikes, digits, preset mapping, rate limiting: see sections above (`processing.rs` `digits`, `sender.rs` `limiter_*`)
- [x] Integration test per preset with exact addresses/types/values: `it`
- [x] svelte-check, eslint, Vitest (stores, formatting), Playwright smoke: `ci.yml` `frontend` job; `src/lib/**/*.test.ts`, `e2e/smoke.spec.ts`
- [x] Rotating structured logs in app data, Copy diagnostics: `src-tauri/src/logging.rs`, Settings > Diagnostics
- [x] No panics in normal operation, typed errors: `core/error.rs` (`KtError`); panic hook logs to file
- [x] CI on every push; release on tags with installer, portable exe, checksums: `ci.yml`, `release.yml`

## Docs
- [x] README: what, screenshots (Playwright, mock data), install, 4-step quick start, avatar setup per preset, FAQ: `README.md`
- [x] docs/PARAMETERS.md, docs/DEVICES.md, DECISIONS.md, MIT `LICENSE`

## Working method
- [x] PLAN.md milestones, commit per milestone
- [x] Full lint/test after each milestone
- [x] This checklist
- [x] Pushed, CI green (see the final report in the session)

## Visual redesign (follow-up)
- [x] All colours as tokens in `src/tokens.css` only; enforced by `src/lib/no-hex.test.ts` (also bans Inter/Roboto/system-ui)
- [x] Bundled fonts (Bricolage Grotesque, Figtree, JetBrains Mono, OFL) in `src/assets/fonts`; overlay serves its own copies
- [x] Sidebar groups, status dots, hairline lists, settings rows, chips vs toggles, monoline heart, 4px zone track, restyled graph and stats: views and components under `src/lib`
- [x] Screenshots dark/light at 1280 and 900 px plus mini and overlay: `e2e/screenshots.spec.ts` -> `docs/screenshots`

## Needs real hardware on Windows
1. Real straps and watches (Polar, Garmin, Coospo, Wahoo, Amazfit/Xiaomi, Galaxy Watch apps): scan, parse, battery, location, reconnect.
2. Radio off / adapter missing / device held by another app messages from WinRT.
3. Idle CPU and RAM, and backend latency on real BLE notifications.
4. Tray icon rendering and menu; start with Windows; start minimized.
5. VRChat OSCQuery discovery and avatar-change resend in a live VRChat session.
6. NSIS install/uninstall and the portable exe on a clean Windows 11 machine.
