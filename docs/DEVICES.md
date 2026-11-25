# Device notes

KittyTooth talks to any sensor that implements the standard Bluetooth **Heart Rate Service
(0x180D)** with the **Heart Rate Measurement (0x2A37)** characteristic. It also reads **Battery
Level (0x2A19)** and **Body Sensor Location (0x2A38)** when present.

> **Testing status.** The parser, processing and OSC pipeline are covered by automated tests
> (every 0x2A37 flag combination, scripted sensors with RR intervals, contact loss, dropouts,
> link loss). KittyTooth was built on a machine without Bluetooth hardware, so the notes below
> are based on the published behaviour of each device family and the GATT specification; the
> per-device behaviour still needs confirmation on real hardware (see the README's "Needs real
> hardware testing" list). Reports are welcome via *Settings → Copy diagnostics*.

## General rules

* **One connection at a time.** Most straps accept a single BLE connection. If Zwift, Polar Beat,
  Garmin Connect, a phone or a bike computer is connected, the strap stops advertising and
  KittyTooth reports *"Sensor not responding"*. Close the other app or turn the phone's
  Bluetooth off. KittyTooth keeps retrying (1, 2, 4, 8, then every 10 s) and connects as soon as
  the strap is free.
* **Don't pair in Windows Settings.** BLE heart rate sensors don't need Windows pairing.
  If a device was paired to Windows for another app and KittyTooth shows *"Sensor is busy"*,
  remove it in *Settings → Bluetooth & devices* and let KittyTooth connect directly.
* **Wake the strap.** Chest straps only advertise while they detect skin contact. Wet the
  electrodes and put it on before scanning.
* **Contact loss.** When a strap reports "no skin contact" KittyTooth keeps the link but marks
  the data invalid after the stale timeout (Connected=false is sent to OSC), and recovers
  automatically when contact returns.

## Families

| Device | RR intervals | Contact flag | Battery | Notes |
|---|---|---|---|---|
| **Polar H10** | yes | yes | yes | Best HRV source. Supports two simultaneous BLE connections in recent firmware, so it can work alongside a phone app. Sends 1-2 RR values per notification. |
| **Polar H9** | yes | yes | yes | Same protocol as H10, single BLE connection. |
| **Polar OH1 / Verity Sense** | yes* | no | yes | Optical. Must be in "HR mode" (not swimming/recording mode). RR derived optically and is noisier; KittyTooth's RR artefact filter handles it, but HRV is less reliable. Reports location *Other*/*Arm*. |
| **Garmin HRM-Dual / HRM-Pro** | yes | yes | varies** | BLE HR service available alongside ANT+. HRM-Pro may only broadcast BLE HR when not paired to a watch over BLE. **Some models report battery only over ANT+. |
| **Garmin watches (Broadcast HR)** | no | no | no | Enable *Broadcast Heart Rate* in the watch's sensor menu. 8-bit HR only; beats are derived from BPM. Broadcasting stops when the activity/broadcast ends. |
| **Coospo H6 / H808S / H9Z** | yes | yes | yes | Standard implementation. Some units send a trailing odd byte in RR data; KittyTooth ignores it. |
| **Wahoo TICKR / TICKR X / FIT** | yes | yes | yes | Standard. TICKR X may report energy expended. Allows up to three BLE connections. |
| **Amazfit / Xiaomi (Mi Band) in HR broadcast mode** | no | no | no | Enable *Heart rate sharing / Broadcast heart rate* in Zepp / Mi Fitness. 8-bit HR, ~1 Hz, often no contact flag; device name may be generic. Mi Bands sometimes repeat the last value while off-wrist; the spike filter and stale timeout limit the impact. |
| **Galaxy Watch HR broadcast apps** | varies | no | no | Third-party HR broadcast apps from the Galaxy Store / Play Store expose a standard 0x180D service. Keep the app in the foreground on the watch; Wear OS may throttle it when the screen is off. |

\* Optical RR, see notes.

## Known quirks KittyTooth handles

* **UINT16 heart rate** (flag bit 0) — supported, values above 250 are rejected as impossible.
* **Sensor contact "supported but not detected"** — readings with HR 0 are ignored, not treated
  as 0 bpm.
* **Empty RR flag** — RR flag set but no intervals: treated as none.
* **Dangling byte** in the RR area — ignored.
* **Single-sample spikes** (e.g. a lone 230 during a run) — held back and discarded unless the
  next sample confirms the jump.
* **Notifications silently stop** (Windows sometimes keeps the link "connected") — the stale
  timeout (default 5 s) tears down the link and reconnects.
* **Windows needs to have seen the device advertise** before connecting — KittyTooth scans
  briefly when reconnecting to a remembered device.
* **Bluetooth radio switched off / adapter unplugged** — reported in plain language; KittyTooth
  polls the adapter and reconnects automatically when it returns.
