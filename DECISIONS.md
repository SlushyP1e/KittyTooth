# Design decisions

Decisions made while building KittyTooth autonomously, with the reasoning behind each.

## Upstream parameter names (verified against source)

| Project | Repository | Commit verified | Files |
|---|---|---|---|
| HRtoVRChat_OSC | github.com/200Tigersbloxed/HRtoVRChat_OSC | `f645496dc3810e43b9ebbfe28733cd9ece99cd30` (main, 2024-01-01) | `HRtoVRChat_OSC/ParamsManager.cs`, `HRtoVRChat_OSC/ConfigManager.cs`, `AvatarSetup.md` |
| VRCOSC | github.com/VolcanicArts/VRCOSC | `d4eff6c451be16564596699245a60a4a6d6128bd` (main, 2026-08-07) | `VRCOSC.App/SDK/Modules/Heartrate/HeartrateModule.cs` |

Findings that shaped the presets:

* **HRtoVRChat**: parameters live at the root of `/avatar/parameters/`: `HR` (int, clamped 0-255),
  `onesHR`/`tensHR`/`hundredsHR` (int), `isHRConnected`, `isHRActive` (bool), `isHRBeat` (bool,
  true for ~100 ms per beat — see the `HeartBeat()` loop in `Program.cs`), `HRPercent` (float,
  `(HR-MinHR)/(MaxHR-MinHR)`, defaults MinHR=0 MaxHR=255) and `FullHRPercent` (float, same mapped to
  -1..1). **`HeartBeatToggle` does not exist in upstream HRtoVRChat_OSC** at that commit (it is
  found in some forks/derived prefabs). Because the spec asks for it, KittyTooth sends it as a bool
  that toggles on every beat; it is harmless for avatars that don't define it. `isHRActive` is not in
  the spec list but *is* upstream, so it is sent too (same value as `isHRConnected`).
* **VRCOSC**: parameters are under `/avatar/parameters/VRCOSC/Heartrate/`. `Enabled` (bool, legacy),
  `Value` (int), `Normalised` (float, default bounds 0-240), `Units`/`Tens`/`Hundreds` are
  **floats equal to digit / 10** (legacy params — not ints), `Beat` (bool, default "toggle" mode).
  Upstream also sends `Connected` (bool) and `Average` (int, 10 s rolling average); KittyTooth sends
  both so newer VRCOSC prefabs work too.
* Each preset's normalisation range defaults to its upstream default (HRtoVRChat 0-255, VRCOSC 0-240,
  KittyTooth 40-200) and can be overridden per target.

## Architecture

* **Core crate without Tauri** (`crates/kittytooth-core`). Parser, processing, engine, OSC,
  OSCQuery, overlay and the mock sensor are all testable on any CI runner. The Tauri crate is a
  thin shell (commands, events, tray, windows, logging).
* **One engine task owns everything time-critical.** BLE notification → parse → process → OSC
  encode → `try_send_to` happens synchronously inside one tokio task, with no UI round-trip and
  no extra channel hop. The UI observes via a `watch` snapshot (forwarded to the webview at most
  ~30 Hz), a broadcast stream for beats/samples, and shared buffers (history, OSC log). Closing or
  hiding the window has no effect on OSC.
* **Latency** is measured from the moment the notification arrives in the backend (`Instant`
  captured in the BLE notification task) to after the UDP send, kept as last/avg/max in the
  snapshot (shown in Settings → Diagnostics) and logged per packet when *Debug logging* is on.
  The integration test asserts avg < 10 ms; locally it is well under 1 ms.
* **Composite sensor backend**: `demo:` ids go to the mock backend, everything else to btleplug.
  The demo strap is therefore available in the real app (toggle in Devices) and exercises the
  exact same pipeline as hardware.
* **Hand-rolled tiny HTTP server** for the overlay and OSCQuery instead of pulling in a web
  framework: only GET + SSE are needed and it keeps idle CPU/RAM minimal.

## Behaviour choices

* **Backoff** 1, 2, 4, 8, 10, 10... seconds (doubling, capped at 10 s as specified), retrying
  forever while the user wants a connection. A successful connection resets it.
* **Two staleness notions.** *Link stale*: no packets at all for the stale timeout → the link
  is torn down and reconnected. *Data stale*: packets arrive but no valid reading (e.g. strap
  reports no skin contact) → OSC `Connected=false`, but the link is kept (reconnecting wouldn't
  help). Short contact blips shorter than the timeout keep the last value.
* **Spike filter**: a jump larger than max(25 bpm, 20 %) from the last accepted value is held
  for one sample; it's accepted if the next sample confirms it, otherwise discarded. Normal
  samples are never delayed. After a gap > 5 s anything in range is accepted directly.
* **Smoothing**: EMA with α = 1 (off), 0.5 (light), 0.25 (medium); reset after a 10 s gap.
  OSC "HR" values use the smoothed value; the raw value is exposed (UI + `hr_raw` metric).
* **RMSSD**: 30 s window; RR outside 240-2400 ms dropped; RR deviating > 25 % from the previous
  accepted one dropped (reset after 3 consecutive to follow genuine rate changes); differences
  are never taken across a data gap > 3 s; needs ≥ 5 intervals.
* **Beat events** replay the sensor's RR intervals (queue of max 4, so latency stays ≤ ~1 beat
  behind) and fall back to 60/BPM when a sensor sends no RR or RR stops for 3 s.
* **Rate limit** is a strict sliding 1 s window (never more than N in any window), not a token
  bucket (which allows bursts of 2N across a boundary). Excess values coalesce in a priority
  queue: beats → connection flags → changed values → keepalive refreshes.
* **Keepalive** re-sends a value only if it hasn't been sent within the keepalive period, so
  changed values don't get sent twice.
* **Disconnect**: connection flags are sent once as false; values hold or zero once (user
  option). No keepalive while disconnected. Quitting the app also sends this frame.
* **Percent** in the KittyTooth preset = BPM / max HR (0..1), distinct from `Normalized` (over
  the configurable min/max). **HRV** is mapped 0-200 ms → 0..1 because VRChat floats are −1..1.
* **Battery** parameter is only sent when the sensor reports a battery level.
* **Zones**: zone 0 below the first threshold (default 50/60/70/80/90 % of max HR).
* **Chatbox** respects the 2 s minimum, sends only when text changes (or every 20 s to remain
  visible) and only while connected; counts against the same per-target rate limit.
* **OSCQuery**: our HTTP and OSC servers bind to loopback only and are advertised with
  127.0.0.1, so Windows Firewall never prompts. VRChat discovery prefers the loopback endpoint;
  a VRChat found on another machine only overrides targets whose host is that machine's IP. We
  expose `/avatar/change` so VRChat notifies us, and re-send every parameter on avatar change.
  mDNS failures are non-fatal (manual ports are used).
* **Config**: `config.json` in the OS app config dir, `schema_version` 2. Version 1 is the flat
  single-target prototype format; `migrate()` upgrades it (and keeps a `config.v1.bak.json`).
  Unknown fields are ignored, missing fields default, values are clamped (`sanitized`). A
  corrupt file is backed up (`config.corrupt-<ts>.json`), defaults are used and the user is told
  once. Saves are atomic (temp file + rename).
* **Closing the main window** hides to the tray by default (setting *Closing the window keeps
  KittyTooth in the tray*), since OSC keeps running. *Start minimized to tray* applies to every
  launch including autostart (`--autostart` flag).
* **Tray icon** renders the BPM digits into a 32×32 bitmap (built-in pixel font) because Windows
  tray icons can't show text; the tooltip has the full status.
* **Scan without an OS service filter**, then filter by advertised services in our code: some
  sensors only put 0x180D in the scan response, which OS filters can miss.
* **Errors**: every fallible path returns `KtError`; the UI only ever receives `UserError`
  (code, title, message, fix). Unknown frontend exceptions become a generic friendly message.
  A panic hook logs panics to the log file instead of losing them.

## Frontend

* Svelte 5 runes, no UI framework; hand-built components. State lives in one `AppStore`
  (`app.svelte.ts`); config edits are debounced (350 ms) and the backend returns the sanitized
  config.
* The graph is a single canvas with an rAF loop that runs only while data is live and the page
  is visible; otherwise it redraws on change or once a second. Points are decimated to ~1 per
  pixel column (min/max), and the y-range eases instead of jumping.
* The heart pulse uses the Web Animations API on each `beat` event (not a CSS loop), so it is
  in sync with real beats; under reduced motion it becomes a subtle opacity change.
* `src/lib/api/mock.ts` simulates the engine in the browser so `npm run dev`, Vitest,
  Playwright and screenshots work without Tauri.

## Visual design ("Slushy")

* Every colour lives in `src/tokens.css` as a custom property; nothing else may contain a hex,
  `rgb()` or `hsl()` literal. `src/lib/no-hex.test.ts` enforces this and bans Inter, Roboto,
  system-ui and Segoe UI as visible fonts. Derived tints use `color-mix()` on tokens.
* Fonts are bundled woff2 (Bricolage Grotesque 500/700/800, Figtree 400/500/600, JetBrains Mono
  400/500, all SIL OFL, licences next to the files). No network font requests. The OBS overlay
  is served by the Rust core, so it embeds its own two fonts (`include_bytes!`) and serves them
  at `/fonts/*`.
* The light theme is derived from the dark tokens: same structure, slate ramp inverted to warm
  near-whites, and the pink darkened to `#c0466f` so text and lines keep contrast on white.
* Accent: `config.ui.accent` is a preset name (`pink`, `peach`, `lilac`, `sky`) or a custom
  `#rrggbb`. Presets switch `data-accent`, whose values (per theme) live in tokens.css. A custom
  hex sets `--pink` inline and derives `--pink-deep`/`--pink-soft` with `color-mix()`. Older
  configs holding a hex keep working as a custom accent; the config default is now `pink`.
* The heart is a monoline outline (pink stroke, 12% pink fill) so it reads as one line drawing;
  the beat is a 1.08 scale with a fading 6px glow driven per beat by WAAPI. Under reduced motion
  a 7px dot blinks instead.
* Zones: one 4px track. (Originally labels-only in pink; superseded by the user's request for
  per-zone colours, see "Zone colours" below.) The graph no longer has a coloured zone strip.
* Toggles only for true on/off; every multi-option setting (smoothing, theme, preset, disconnect
  behaviour, graph range) is a chip group or tab set with radiogroup semantics and arrow keys.
* The tray icon draws charcoal digits on the accent colour (accent presets map to their dark
  theme values in `tray_icon::accent_rgb`); idle is a slate tile with light digits.

## Heart rate sources (Pulsoid, HypeRate, OSC input, fallback)

### Researched API facts (read 2026-11-25)

**Pulsoid**, implemented to the official docs:
* Real-time v2: <https://docs.pulsoid.net/heart-rate-and-sensor-data/read-data-via-websocket-v2>.
  URL `wss://dev.pulsoid.net/api/v2/data/real_time`. The token goes in the `access_token` query
  parameter (or `Authorization: Bearer`). `types` is a comma-separated list of `heart_rate`
  (scope `data:heart_rate:read`), `battery_level` (`data:battery_level:read`) and
  `ingestion_pause`. Messages: `{"type","timestamp" (ISO 8601),"heartRate":{"bpm"}}`,
  `{"batteryLevel":{"percent"}}`, `{"ingestionPause":{"id","started_at","ends_at"}}`. Upgrade
  statuses: 101 OK, 400 when the token lacks a scope for a requested type (error 6003), 401 when
  the token is missing or invalid. The server closes the socket when the token is revoked or
  expires. The docs say the connection "can be interrupted at any point" and to retry with
  back-off.
* v1 (legacy): <https://docs.pulsoid.net/heart-rate-and-sensor-data/read-heart-rate-via-websocket>,
  `{"measured_at": ms, "data": {"heart_rate": n}}`. The parser accepts it too.
* Tokens: manual tokens come from <https://pulsoid.net/ui/keys> ("Create new token"), which is a
  BRO plan feature (paid or trial)
  (<https://docs.pulsoid.net/getting-started/access-token-management/manual-token-issuing>).
  Validation is `GET https://dev.pulsoid.net/api/v1/token/validate` with a Bearer header,
  returning `{token, client_id, expires_in, profile_id, scopes}`
  (<https://docs.pulsoid.net/getting-started/access-token-management/validate-authorization-token>).
* Errors: <https://docs.pulsoid.net/reference/error-code-format>. The body is
  `{"error_code","error_message"}`; for example 7005 `token_not_found` (401), 7006
  `token_expired` (401), 7007 `premium_required` (402), 6003 `error_invalid_scope` (400).
* Rate limits: **not documented** anywhere in the docs.

**HypeRate**, implemented to the official WebSocket API document
(<https://github.com/HypeRate/HypeRate-Websocket-API/blob/main/docs/WEBSOCKET_API.md>). It is
linked from <https://developers.hyperate.io/>, whose live demo uses the same protocol.
* URL `wss://app.hyperate.io/ws/:deviceId?token=API_KEY`: the device ID goes in the path and the
  developer key in `token`.
* After opening, send `{"topic":"hr:ID","event":"phx_join","payload":{},"ref":"1"}`. Updates
  arrive as `{"topic":"hr:ID","event":"hr_update","payload":{"hr":75},"ref":null}`. Send the
  heartbeat `{"event":"ping","payload":{"timestamp":ms}}` every **15 s**, and `phx_leave`
  before closing. `phx_reply` "can safely be ignored".
* Keys are requested at <https://www.hyperate.io/api> (free tier for research and indie
  projects). The docs say never to commit or share keys.
* **Not documented**: rate limits, connection limits, the device ID format, how an invalid key
  or unknown device is reported, and the reply formats.

### Choices where the docs are silent
* Pulsoid: the source requests `heart_rate,battery_level`. If the token lacks the battery scope
  (HTTP 400), it reconnects with `heart_rate` only. Upgrade 401/403 means bad token, 402 means
  plan required, 429 means rate limited, and any other status is a temporary service error.
* HypeRate: upgrade 401/403 means bad API key, 404 means unknown device, and 429 means rate
  limited. A `phx_reply` with `status: "error"` to our join (ref "1") also means the device ID was
  refused. Device IDs are accepted only as 2-32 characters of `[A-Za-z0-9_-]`, so they are safe
  in a URL path.
* Rate limits: since neither service documents limits, KittyTooth keeps one connection per
  source and uses the Bluetooth back-off (1, 2, 4, 8, 10 s). An HTTP 429 raises the next delay
  to at least 30 s.
* Rejected credentials or IDs (bad token or key, plan required, invalid device ID, missing
  setup) are not retried, matching Pulsoid's own client, which stops on unauthorized. The
  source stays idle with a plain-language error until the user saves a new secret or changes
  settings (`RestartSources`).
* Silence: network sources may only send on change, so their link is torn down after
  `max(stale timeout, 20 s)` without messages. Pulsoid's official client treats 30 s of silence
  as offline. Data validity (OSC `Connected`) still uses the normal stale timeout. The OSC
  input has no link, so silence only marks data stale.

### Architecture
* `HeartRateSource` (`crates/kittytooth-core/src/source/`): `kind()`, `capabilities()`,
  `label()` and `connect(events)`. Every source emits `SensorEvent::Sample(HrSample)`, where
  `HrSample` carries bpm, RR (ms), battery, contact (`Option<bool>`), energy, timestamp,
  received instant and source kind. Bluetooth and demo decode `0x2A37` into `HrSample` inside
  the session, so the engine never sees raw bytes. A `SourceFactory` builds sources from config
  and secrets; tests swap the endpoints for a local mock server.
* The engine runs two **slots**, main and backup, each with its own session, reconnect loop
  and error. Exactly one slot is live, and only its samples reach the processor and OSC. The
  standby slot's samples only update its own freshness. Switching sources is a command
  (`SelectSource`), never a restart.
* Fallback switches to the backup once the main source has had no valid reading for
  `after_secs` (default 5) and the backup is delivering. It switches back once the main source
  has delivered steadily for 3 s, to avoid flapping. A switch resets the spike filter and beat
  scheduler, keeps session stats, and sends `Connected=false` only if the new live source has
  no data. The backup stays connected ("hot standby") so takeover is instant.
* Capabilities: Bluetooth and demo give RR, battery and contact; Pulsoid gives battery;
  HypeRate and OSC give heart rate only. The UI shows missing items as "not available from this
  source", never 0. OSC keeps its documented behaviour: Battery is omitted and HRV is 0.

### Secrets
* Tokens and keys go to the OS credential store through `keyring` v3: Windows Credential Manager
  (`windows-native`), macOS Keychain, and Secret Service on Linux. Service `KittyTooth`,
  accounts `pulsoid-token` and `hyperate-api-key`. The core only sees a `SecretStore` trait
  (memory-backed in tests).
* Never in `config.json`, never logged (URLs carrying tokens are not logged, and
  `tungstenite`/`native_tls` stay at `warn` even in debug mode), masked in the UI (`••••` plus
  the last 4 characters), and absent from Copy diagnostics, which also scrubs any stored value
  as a safeguard.
* HypeRate developer key: read at compile time from `KITTYTOOTH_HYPERATE_KEY` (`option_env!`),
  which CI fills from the `HYPERATE_API_KEY` repository secret. A user override in the
  credential store takes precedence. No key is in the repository.
* TLS for WebSockets uses `native-tls`: SChannel on Windows (no extra build tools) and OpenSSL
  on Linux (`libssl-dev` is already a CI dependency).

### Zone colours (user request)
* The user asked for the multi-colour zone bar back. Zones now have their own pastel tokens
  (`--zone-0..5` in `tokens.css`, deeper values in the light theme). The current zone is shown at
  full strength and slightly thicker; the others are at 40 %. This deliberately overrides the
  redesign's single-accent rule for this one data encoding. The graph stays pink-only.

### Saved session history
* One append-only text file per session (`<start ms>.kts` in the local app data folder) plus a
  JSON summary written when the session ends, instead of SQLite. No C toolchain or new
  dependency, a crash loses at most the line being written, files without a summary are finished
  on the next start, and a session is easy to inspect or delete by hand. About 70 KB per hour.
* Recording rules live in a pure `history::Recorder` (start on the first valid reading, about one
  sample per second, end after 3 quiet minutes, Reset session, quit or turning recording off,
  drop sessions under 30 s) so they are unit tested with plain timestamps. Disk writes run on a
  dedicated thread fed by a channel, so the OSC hot path never waits on the disk.
* Zone settings are copied into each session's header so its zone times don't change when the
  settings do. Min/avg/max use the raw BPM; the graph shows the displayed (smoothed) BPM.
* Test alerts and test sweeps are never recorded. Everything stays on the PC; export is only
  through the save dialog.

### Also fixed
* OSC output now uses plain non-blocking `std::net::UdpSocket` sends. Tokio's `try_send_to`
  can return `WouldBlock` on a freshly bound socket before readiness is first observed, which
  dropped the first few OSC messages under load.

## In-app updates

* `tauri-plugin-updater` polls `latest.json` from the latest GitHub release on launch and
  every 6 hours. The check and install run in Rust commands (`src-tauri/src/updates.rs`), not
  the plugin's JS API, so the version rule (stable builds never get pre-releases) and the
  portable vs installed decision live in `kittytooth-core::update` with unit tests.
* The release workflow builds with `tauri build` directly, not tauri-action, so the publish
  job writes `latest.json` itself from the `.sig` files. Signing is optional: without the
  `TAURI_SIGNING_PRIVATE_KEY` secret the release still ships, minus the update files.
* Portable copies are detected by the missing NSIS `uninstall.exe` next to the exe (Linux:
  no `$APPIMAGE`). They get a release page link, since a running exe can't replace itself.
  Until a real public key replaces the placeholder in `tauri.conf.json`, every copy behaves
  this way.
* Installing needs a click, and a second one while heart rate is live. Release notes are
  shown as plain text, never as HTML.
## SteamVR overlay and haptics
* **Binding: load SteamVR's `vrclient` at runtime with `libloading`, no SDK crate.** The `openvr`
  / `openvr_sys` crates build the OpenVR C++ SDK with cmake and run bindgen (libclang), and
  `ovr_overlay` needs autocxx (also libclang); all are heavier on CI and some lag the SDK.
  Instead `steamvr/ffi.rs` does what `openvr_api.dll` does: read `openvrpaths.vrpath`
  (`%LOCALAPPDATA%\openvr\` or `~/.config/openvr/`), load `bin/vrclient_x64.dll` or
  `bin/linux64/vrclient.so`, get `IVRClientCore_003` from `VRClientCoreFactory`, then the C
  function tables `FnTable:IVRSystem_026` and `FnTable:IVROverlay_028`. Struct layouts and table
  indices were taken from OpenVR 2.15.6 (`openvr_capi.h`, `src/ivrclientcore.h`). Nothing is needed
  at build time, so CI builds it on Windows and Linux as is; no feature flag.
* **Never launches SteamVR.** It first initialises as a *background* app, which fails with
  `NoServerForBackgroundApp` (121) when SteamVR isn't running, then reconnects as an *overlay*
  app. It probes every 5 s (15 s if not installed) and acknowledges `VREvent_Quit` so SteamVR can
  close.
* **Own OS thread**, reading only the engine's watch/broadcast channels; a slow SteamVR can't
  touch the OSC path. A panic there sets an error status instead of taking the app down.
* **Overlay image drawn in Rust** (`steamvr/render.rs`, 512x256 RGBA uploaded with
  `SetOverlayRaw` at most every 250 ms and only when it changes). The bundled fonts are woff2,
  which would need a decoder, so digits and the few labels (BPM, HIGH, LOW, TEST) are rounded
  strokes with distance-field anti-aliasing, and the logo is the cat-head path from `Logo.svelte`.
* **No flicker on updates: two overlays, double-buffered.** Re-uploading one overlay with
  `SetOverlayRaw` blanked it for a frame in the headset. The new image now goes to a hidden twin
  overlay at the same pose, which is shown with a higher sort order; the old one is hidden 60 ms
  later (a few compositor frames). Uploads happen only when the rendered pixels differ, at most
  every 250 ms.
* **Fade when not looking** (wrist): every ~25 ms the thread reads the HMD and controller poses
  (`GetDeviceToAbsoluteTrackingPose`), works out the overlay's world pose (controller x attach
  offset), and maps the angle from the view centre (smoothstep between 20° and 38° for "normal")
  and the card's facing angle to a target alpha, eased over 0.2 s and multiplied by the opacity
  setting. Active alerts force full visibility. The math is in `steamvr/fade.rs` with unit tests.
* **Haptics use the legacy `TriggerHapticPulse`** (pulses of up to 4 ms, one per 5 ms, played as
  a 90 ms + 70 ms "lub-dub"). The SteamVR Input alternative needs an action manifest and default
  bindings per controller type.

## Stack deviations

* None. Tauri 2, btleplug, rosc, tokio, serde, Svelte 5 + TS + Vite as specified. Additions:
  `mdns-sd` (OSCQuery mDNS), `tracing` + `tracing-appender` (rotating logs, 7 daily files),
  Tauri plugins `single-instance`, `autostart`, `dialog`, `clipboard-manager`, `opener`,
  `notification`, `updater`; `semver` for update version checks;
  `tokio-tungstenite` (native-tls) for the Pulsoid/HypeRate WebSockets, `keyring` for
  secrets and `libloading` (SteamVR's client, loaded at runtime).
* `@playwright/test` is pinned to 1.63.0 to match the browser build available in the
  development environment; CI installs the matching Chromium.
