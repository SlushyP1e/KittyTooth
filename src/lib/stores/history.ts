import type { HistoryPoint } from "../types";

/**
 * Append-only time series for the live graph. Stores timestamps and values in flat number
 * arrays (no per-frame allocation when drawing) and supports binary-searched range queries.
 */
export class HistoryBuffer {
  readonly t: number[] = [];
  readonly v: number[] = [];
  private readonly cap: number;
  /** Increments on every change, so views can cheaply detect updates. */
  version = 0;

  constructor(cap = 43_200) {
    this.cap = cap;
  }

  get length(): number {
    return this.t.length;
  }

  push(p: HistoryPoint): void {
    const last = this.t.length ? this.t[this.t.length - 1] : -Infinity;
    if (p.t_ms < last) return; // out of order: ignore
    if (p.t_ms === last) {
      this.v[this.v.length - 1] = p.bpm;
    } else {
      this.t.push(p.t_ms);
      this.v.push(p.bpm);
    }
    if (this.t.length > this.cap) {
      const drop = this.t.length - this.cap;
      this.t.splice(0, drop);
      this.v.splice(0, drop);
    }
    this.version++;
  }

  set(points: HistoryPoint[]): void {
    this.t.length = 0;
    this.v.length = 0;
    const sorted = [...points].sort((a, b) => a.t_ms - b.t_ms);
    // Trim first: pushing past the cap would shift the whole array once per extra point.
    for (const p of sorted.slice(-this.cap)) this.push(p);
    this.version++;
  }

  clear(): void {
    this.t.length = 0;
    this.v.length = 0;
    this.version++;
  }

  /** Index of the first point with t >= ms. */
  indexAtOrAfter(ms: number): number {
    let lo = 0;
    let hi = this.t.length;
    while (lo < hi) {
      const mid = (lo + hi) >>> 1;
      if (this.t[mid] < ms) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  lastTime(): number | null {
    return this.t.length ? this.t[this.t.length - 1] : null;
  }

  firstTime(): number | null {
    return this.t.length ? this.t[0] : null;
  }

  /** Min and max value in [from, to] (inclusive indices), or null when empty. */
  extent(from: number, to: number): [number, number] | null {
    if (from > to || from >= this.v.length) return null;
    let mn = Infinity;
    let mx = -Infinity;
    for (let i = Math.max(0, from); i <= Math.min(to, this.v.length - 1); i++) {
      const x = this.v[i];
      if (x < mn) mn = x;
      if (x > mx) mx = x;
    }
    return mn === Infinity ? null : [mn, mx];
  }
}
