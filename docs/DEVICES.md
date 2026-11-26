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
| **Garmin watches (Broadcast HR)** | no (BLE) / yes (ANT+) | no | no | Enable *Broadcast Heart Rate* in the watch's sensor menu. Over Bluetooth: 8-bit HR only; beats are derived from BPM. Over ANT+ the watch also sends beat times, so use the **ANT+** source for HRV (see below). Broadcasting stops when the activity/broadcast ends. |
| **Coospo H6 / H808S / H9Z** | yes | yes | yes | Standard implementation. Some units send a trailing odd byte in RR data; KittyTooth ignores it. |
| **Wahoo TICKR / TICKR X / FIT** | yes | yes | yes | Standard. TICKR X may report energy expended. Allows up to three BLE connections. |
| **Amazfit / Xiaomi (Mi Band) in HR broadcast mode** | no | no | no | Enable *Heart rate sharing / Broadcast heart rate* in Zepp / Mi Fitness. 8-bit HR, ~1 Hz, often no contact flag; device name may be generic. Mi Bands sometimes repeat the last value while off-wrist; the spike filter and stale timeout limit the impact. |
| **Galaxy Watch HR broadcast apps** | varies | no | no | Third-party HR broadcast apps from the Galaxy Store / Play Store expose a standard 0x180D service. Keep the app in the foreground on the watch; Wear OS may throttle it when the screen is off. |

\* Optical RR, see notes.

## ANT+

The **ANT+** source reads any ANT+ Heart Rate Monitor (device profile 120) through a USB ANT
stick. KittyTooth opens one wildcard channel (RF 57, period 8070, the ANT+ network key) and
pairs with the first monitor it hears.

> **Testing status.** The ANT message protocol, HRM page parsing and RR derivation are covered by
> automated tests with byte fixtures and a scripted stick, including beat-count gaps and the
> 16-bit event-time wraparound. It has not been confirmed on a real stick yet; reports via
> *Settings → Copy diagnostics* are welcome.

**Sticks.** Garmin/Dynastream **ANT USB2** (USB ID `0fcf:1008`) and **ANT USB-m** (`0fcf:1009`),
and clones that reuse those IDs (common for CooSpo, CYCPLUS and Anself sticks).
The original **ANT USB1** stick (`0fcf:1004`) is a serial (CP210x) device and isn't supported.
Several sticks can be plugged in; KittyTooth uses the first one no other program holds.

**Drivers.**

* **Windows**: KittyTooth needs the stick on the **WinUSB** driver. If it says *"ANT+ stick needs
  a different driver"*, run [Zadig](https://zadig.akeo.ie), choose *Options → List All Devices*,
  pick *ANT USB Stick 2* / *ANT USB-m Stick*, select **WinUSB** and press *Replace Driver*, then
  unplug and replug the stick. To undo it, uninstall the device in Device Manager (tick *delete
  the driver*) and replug.
* **Linux**: the kernel's usb-serial driver is detached automatically. Your user needs access to
  the device, e.g. `/etc/udev/rules.d/99-ant.rules` with
  `SUBSYSTEM=="usb", ATTRS{idVendor}=="0fcf", ATTRS{idProduct}=="100[89]", MODE="0666"`.

**One program at a time.** A stick can only be opened by one program. Close Garmin Express,
Zwift, TrainerRoad, Golden Cheetah and similar apps (check the system tray) or KittyTooth reports
*"ANT+ stick is in use"* and keeps retrying.

**What you get.**

| Data | Notes |
|---|---|
| Heart rate | Computed HR from every page (0 = no reading, ignored). Sent to KittyTooth about once a beat. |
| RR intervals / HRV | From consecutive heart beat event times (bytes 4-5, 1/1024 s) when the beat count goes up by exactly 1. When beats were missed, page 4's previous-beat time still gives the last interval. Works on Garmin watches that send no RR over Bluetooth. |
| Battery | From page 7 when the monitor reports a percentage. Many straps only report a status (good/low), which isn't shown. |
| Skin contact | Not part of the ANT+ HRM profile; shown as *not available from this source*. |

**Reconnects.** Unplugging the stick is reported as *"No ANT+ stick found"* and KittyTooth
reconnects when it's back. If no monitor is heard for 20 s, KittyTooth reports *"Sensor not
found"* and keeps searching. If the monitor goes quiet, the stale timeout reconnects as with
Bluetooth.

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
