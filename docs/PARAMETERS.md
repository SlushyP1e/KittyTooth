# OSC parameters

Every address KittyTooth sends, per preset. All avatar parameters are sent as individual OSC
messages (no bundles) to each enabled target (default `127.0.0.1:9000`).

**Sending rules (all presets)**

* A value is sent when it changes, and re-sent as a keepalive if it hasn't been sent for
  `keepalive_ms` (default 1000 ms; 0 disables keepalive).
* Hard cap: never more than `max_rate` messages in any 1-second window per target
  (default and maximum 20). When the budget is tight, beat parameters go first, then
  connection flags, then changed values, then keepalive refreshes; queued values coalesce to
  the latest value.
* When the sensor disconnects (link lost, or no valid reading for the stale timeout, default
  5 s) the connection flags are sent as `false` **once**. Other values either keep their last
  value (*Hold last value*, default) or are sent once as 0 (*Send 0*).
* When VRChat reports an avatar change via OSCQuery (`/avatar/change`), every parameter is
  re-sent immediately.
* "Normalize range" defaults per preset (shown below) and can be overridden per target.
* Every heart rate source (Bluetooth, Pulsoid, HypeRate, OSC input, demo) drives the same
  parameters. With fallback on, the parameters follow whichever source is live; switching
  doesn't send a disconnect unless neither source has data.
* Data a source can't provide: **Battery** is not sent at all; **HRV** is sent as 0 (Pulsoid,
  HypeRate and OSC input carry no RR intervals). Beats are timed from BPM for those sources.

## 1. KittyTooth (native)

Prefix: `/avatar/parameters/KittyTooth/`

| Address | Type | Range | Meaning |
|---|---|---|---|
| `/avatar/parameters/KittyTooth/HR` | int | 0-250 | Current BPM (smoothed if smoothing is on) |
| `/avatar/parameters/KittyTooth/Normalized` | float | 0..1 | BPM mapped over the target's range (default 40-200), clamped |
| `/avatar/parameters/KittyTooth/Percent` | float | 0..1 | BPM / max heart rate (220 − age, or manual), clamped |
| `/avatar/parameters/KittyTooth/Ones` | int | 0-9 | Ones digit |
| `/avatar/parameters/KittyTooth/Tens` | int | 0-9 | Tens digit |
| `/avatar/parameters/KittyTooth/Hundreds` | int | 0-2 | Hundreds digit |
| `/avatar/parameters/KittyTooth/Connected` | bool | | Valid data within the stale timeout |
| `/avatar/parameters/KittyTooth/Beat` | bool | | Toggles on every heartbeat |
| `/avatar/parameters/KittyTooth/BeatPulse` | bool | | `true` for 100 ms on every heartbeat |
| `/avatar/parameters/KittyTooth/Zone` | int | 0-5 | Heart rate zone (0 = below zone 1) |
| `/avatar/parameters/KittyTooth/HRV` | float | 0..1 | RMSSD over 30 s, 0-200 ms mapped to 0..1 (0 when unavailable) |
| `/avatar/parameters/KittyTooth/Battery` | float | 0..1 | Sensor battery (only sent if the sensor reports it) |
| `/avatar/parameters/KittyTooth/Alert` | int | 0-2 | Heart rate alert: 0 none, 1 high, 2 low (Settings → Alerts; *Test alert* sends 1 for 8 s) |

## 2. HRtoVRChat compatible

Prefix: `/avatar/parameters/` (root). Default range 0-255 (upstream `MinHR`/`MaxHR`).

| Address | Type | Range | Meaning |
|---|---|---|---|
| `/avatar/parameters/HR` | int | 0-255 | Current BPM, clamped to 255 like upstream |
| `/avatar/parameters/onesHR` | int | 0-9 | Ones digit |
| `/avatar/parameters/tensHR` | int | 0-9 | Tens digit |
| `/avatar/parameters/hundredsHR` | int | 0-2 | Hundreds digit |
| `/avatar/parameters/isHRConnected` | bool | | Connected |
| `/avatar/parameters/isHRActive` | bool | | Connected (upstream sends both) |
| `/avatar/parameters/isHRBeat` | bool | | `true` for ~100 ms per beat (upstream semantics) |
| `/avatar/parameters/HeartBeatToggle` | bool | | Toggles every beat (see DECISIONS.md) |
| `/avatar/parameters/HRPercent` | float | 0..1 | `(HR − min) / (max − min)`, clamped |
| `/avatar/parameters/FullHRPercent` | float | −1..1 | `HRPercent × 2 − 1` |

## 3. VRCOSC Heartrate compatible

Prefix: `/avatar/parameters/VRCOSC/Heartrate/`. Default range 0-240 (upstream
`NormalisedLowerbound`/`NormalisedUpperbound`).

| Address | Type | Range | Meaning |
|---|---|---|---|
| `/avatar/parameters/VRCOSC/Heartrate/Enabled` | bool | | Connected (legacy) |
| `/avatar/parameters/VRCOSC/Heartrate/Connected` | bool | | Connected (current upstream) |
| `/avatar/parameters/VRCOSC/Heartrate/Value` | int | 0-250 | Current BPM |
| `/avatar/parameters/VRCOSC/Heartrate/Normalised` | float | 0..1 | BPM over range, clamped |
| `/avatar/parameters/VRCOSC/Heartrate/Average` | int | 0-250 | 10-second rolling average |
| `/avatar/parameters/VRCOSC/Heartrate/Units` | float | 0..0.9 | Ones digit ÷ 10 (upstream legacy format) |
| `/avatar/parameters/VRCOSC/Heartrate/Tens` | float | 0..0.9 | Tens digit ÷ 10 |
| `/avatar/parameters/VRCOSC/Heartrate/Hundreds` | float | 0..0.2 | Hundreds digit ÷ 10 |
| `/avatar/parameters/VRCOSC/Heartrate/Beat` | bool | | Toggles every beat (upstream default "toggle" mode) |

## 4. Custom

Each mapping is *address* + *value* + *type*. A bare name like `MyHR` becomes
`/avatar/parameters/MyHR`; anything starting with `/` is used as-is. Addresses may not contain
spaces or `# * , ? [ ] { }`.

| Value | Raw range | Notes |
|---|---|---|
| Heart rate (bpm) | 0-250 | |
| Heart rate, raw | 0-250 | Unsmoothed |
| Normalized 0..1 | 0..1 | Over the target's range |
| Normalized −1..1 | −1..1 | |
| Percent of max HR | 0..1 | |
| Ones / Tens / Hundreds digit | 0-9 | |
| Connected | 0/1 | |
| Beat (toggle) | 0/1 | |
| Beat (100 ms pulse) | 0/1 | |
| Zone | 0-5 | |
| HRV (ms) | 0-~300 | RMSSD |
| HRV 0..1 | 0..1 | 0-200 ms |
| Battery 0..1 | 0..1 | |
| Session min / average / max | 0-250 | |
| Alert | 0-2 | 0 none, 1 high, 2 low |

Type conversion: **int** rounds, **float** is the raw value, **bool** is `true` when the value is
≥ 0.5.

## VRChat chatbox

`/chatbox/input` with arguments `(string text, bool true, bool notify)`:

* `text` from the template (max 144 characters). Tokens: `{hr}` `{heart}` `{min}` `{avg}` `{max}`
  `{hrv}` `{zone}` `{battery}`.
* `true` = send immediately (don't open the keyboard).
* `notify` = `false` when *Silent* is on (no notification sound).
* Sent at most every 2 s (configurable upward), only when the text changes or every 20 s to stay
  visible, and only while connected.

## OSC input (receiving)

The *OSC input* source listens for a heart rate from another app and relays it through the
presets above.

| Setting | Default | Notes |
|---|---|---|
| Port | `9010` | UDP. Must differ from every output target port, or KittyTooth would hear itself. |
| Address | `/avatar/parameters/HR` | A bare name like `HR` gets the `/avatar/parameters/` prefix. |
| Accept from other devices | off | Off listens on `127.0.0.1` only; on listens on all interfaces. |

Accepted values: `int`, `long`, `float` or `double`, taken as BPM (floats are rounded).
Bundles are searched for the address; the last matching value wins. Values outside 25-250
are rejected like any other reading.

