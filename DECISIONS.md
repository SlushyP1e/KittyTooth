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
