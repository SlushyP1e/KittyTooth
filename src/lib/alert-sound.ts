// A short two-tone chime for heart rate alerts, made with Web Audio (no sound files).
// High alerts rise, low alerts fall, so the two can be told apart without looking.

import type { AlertKind } from "./types";

let ctx: AudioContext | null = null;

export function playAlertSound(kind: AlertKind, volume = 0.18): void {
  try {
    const AC = window.AudioContext ?? (window as unknown as { webkitAudioContext?: typeof AudioContext }).webkitAudioContext;
    if (!AC) return;
    ctx ??= new AC();
    if (ctx.state === "suspended") void ctx.resume();
    const notes = kind === "high" ? [660, 880, 1100] : [880, 660, 520];
    const t0 = ctx.currentTime + 0.02;
    notes.forEach((f, i) => {
      const c = ctx as AudioContext;
      const osc = c.createOscillator();
      const gain = c.createGain();
      osc.type = "sine";
      osc.frequency.value = f;
      const t = t0 + i * 0.16;
      gain.gain.setValueAtTime(0, t);
      gain.gain.linearRampToValueAtTime(volume, t + 0.015);
      gain.gain.exponentialRampToValueAtTime(0.0001, t + 0.3);
      osc.connect(gain).connect(c.destination);
      osc.start(t);
      osc.stop(t + 0.32);
    });
  } catch {
    // Audio is a nicety; never let it break the app.
  }
}
