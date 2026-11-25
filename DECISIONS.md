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
* Zones are distinguished by labels, not hues: one 4px track, current zone pink, lower zones pink
  at 35%. The graph no longer has a coloured zone strip.
* Toggles only for true on/off; every multi-option setting (smoothing, theme, preset, disconnect
  behaviour, graph range) is a chip group or tab set with radiogroup semantics and arrow keys.
* The tray icon draws charcoal digits on the accent colour (accent presets map to their dark
  theme values in `tray_icon::accent_rgb`); idle is a slate tile with light digits.

## Stack deviations

* None. Tauri 2, btleplug, rosc, tokio, serde, Svelte 5 + TS + Vite as specified. Additions:
  `mdns-sd` (OSCQuery mDNS), `tracing` + `tracing-appender` (rotating logs, 7 daily files),
  Tauri plugins `single-instance`, `autostart`, `dialog`, `clipboard-manager`.
* `@playwright/test` is pinned to 1.63.0 to match the browser build available in the
  development environment; CI installs the matching Chromium.
