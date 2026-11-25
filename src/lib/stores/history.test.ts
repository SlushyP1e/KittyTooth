import { describe, expect, it } from "vitest";
import { HistoryBuffer } from "./history";

const p = (t: number, bpm: number) => ({ t_ms: t, bpm, raw: bpm });

describe("HistoryBuffer", () => {
  it("appends in order and bumps version", () => {
    const h = new HistoryBuffer();
    const v0 = h.version;
    h.push(p(1000, 60));
    h.push(p(2000, 62));
    expect(h.length).toBe(2);
    expect(h.version).toBeGreaterThan(v0);
    expect(h.firstTime()).toBe(1000);
    expect(h.lastTime()).toBe(2000);
  });

  it("ignores out-of-order points and replaces duplicates", () => {
    const h = new HistoryBuffer();
    h.push(p(2000, 60));
    h.push(p(1000, 99));
    h.push(p(2000, 65));
    expect(h.length).toBe(1);
    expect(h.v[0]).toBe(65);
  });

  it("caps its size, dropping the oldest", () => {
    const h = new HistoryBuffer(3);
    for (let i = 0; i < 5; i++) h.push(p(i * 1000, 60 + i));
    expect(h.length).toBe(3);
    expect(h.t[0]).toBe(2000);
    expect(h.v[2]).toBe(64);
  });

  it("finds range starts by binary search", () => {
    const h = new HistoryBuffer();
    for (let i = 0; i < 100; i++) h.push(p(i * 1000, 60));
    expect(h.indexAtOrAfter(-5)).toBe(0);
    expect(h.indexAtOrAfter(50_000)).toBe(50);
    expect(h.indexAtOrAfter(50_500)).toBe(51);
    expect(h.indexAtOrAfter(1e9)).toBe(100);
  });

  it("computes value extents", () => {
    const h = new HistoryBuffer();
    [70, 65, 90, 80].forEach((v, i) => h.push(p(i, v)));
    expect(h.extent(0, 3)).toEqual([65, 90]);
    expect(h.extent(2, 3)).toEqual([80, 90]);
    expect(h.extent(5, 9)).toBeNull();
    expect(new HistoryBuffer().extent(0, 0)).toBeNull();
  });

  it("set() sorts and clear() empties", () => {
    const h = new HistoryBuffer();
    h.set([p(3, 3), p(1, 1), p(2, 2)]);
    expect(h.t).toEqual([1, 2, 3]);
    h.clear();
    expect(h.length).toBe(0);
  });
});
