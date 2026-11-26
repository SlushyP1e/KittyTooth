import { afterEach, describe, expect, it, vi } from "vitest";
import { defaultConfig } from "./defaults";
import { csvFileName, formatBytes, formatDay, formatSpan, sessionCsv, sourceLabel, zoneOf } from "./sessions";
import { createMockBackend, type MockBackend } from "./api/mock";
import { mockSessions } from "./api/mock-sessions";
import type { Session } from "./types";

let mock: MockBackend | null = null;
afterEach(() => {
  mock?.dispose();
  mock = null;
  vi.restoreAllMocks();
});

describe("session helpers", () => {
  it("puts heart rates in the same zones as the engine", () => {
    const z = defaultConfig().zones; // max 190, thresholds 50/60/70/80/90 %
    expect(zoneOf(70, z)).toBe(0);
    expect(zoneOf(95, z)).toBe(1);
    expect(zoneOf(120.5, z)).toBe(2);
    expect(zoneOf(171, z)).toBe(5);
    expect(zoneOf(171, { ...z, age: 30 })).toBe(5);
  });

  it("formats spans, sizes and labels", () => {
    expect(formatSpan(50_000)).toBe("50 s");
    expect(formatSpan(42 * 60_000)).toBe("42 min");
    expect(formatSpan(108 * 60_000)).toBe("1 h 48 min");
    expect(formatSpan(120 * 60_000)).toBe("2 h");
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(70_000)).toBe("68 KB");
    expect(formatBytes(3 * 1024 * 1024)).toBe("3.0 MB");
    expect(sourceLabel({ device: null, source: "pulsoid" })).toBe("Pulsoid");
    expect(sourceLabel({ device: "Polar H10", source: "ble" })).toBe("Polar H10");
    const now = new Date(2026, 10, 26, 12).getTime();
    expect(formatDay(new Date(2026, 10, 26, 8).getTime(), now)).toBe("Today");
    expect(formatDay(new Date(2026, 10, 25, 23).getTime(), now)).toBe("Yesterday");
  });

  it("exports the same CSV as the app", () => {
    const start = Date.UTC(2026, 10, 26, 14, 3, 5);
    const zones = defaultConfig().zones;
    const s: Session = {
      summary: {
        id: String(start),
        start_ms: start,
        end_ms: start + 2000,
        source: "ble",
        device: null,
        min_bpm: 70,
        avg_bpm: 120,
        max_bpm: 171,
        samples: 3,
        alerts: 1,
        has_rr: true,
        active: false,
        bytes: 0,
      },
      zones,
      max_hr: 190,
      samples: [
        { t_ms: start, bpm: 70, raw: 70, rr_ms: [857] },
        { t_ms: start + 1000, bpm: 120.5, raw: 121, rr_ms: [500, 498] },
        { t_ms: start + 2000, bpm: 171, raw: 171, rr_ms: [] },
      ],
      alerts: [{ t_ms: start + 1500, kind: "high", bpm: 171, threshold: 160 }],
      zone_secs: [0, 0, 0, 0, 0, 0],
      hrv: [],
    };
    // Same expected lines as the Rust test `history::tests::csv_format`.
    expect(sessionCsv(s).trimEnd().split("\n")).toEqual([
      "timestamp,elapsed_s,bpm,raw,rr_ms,zone,alert",
      "2026-11-26T14:03:05.000Z,0.0,70.0,70,857,0,",
      "2026-11-26T14:03:06.000Z,1.0,120.5,121,500 498,2,",
      "2026-11-26T14:03:07.000Z,2.0,171.0,171,,5,high 171",
    ]);
    expect(csvFileName(start)).toBe("kittytooth-2026-11-26-1403.csv");
  });
});

describe("mock sessions", () => {
  it("are deterministic, plausible and consistent with their summaries", () => {
    const now = new Date(2026, 10, 26, 12).getTime();
    const a = mockSessions(now);
    const b = mockSessions(now);
    expect(a.map((s) => s.summary)).toEqual(b.map((s) => s.summary));
    expect(a).toHaveLength(4);
    for (const s of a) {
      const raws = s.samples.map((p) => p.raw);
      expect(s.summary.min_bpm).toBe(Math.min(...raws));
      expect(s.summary.max_bpm).toBe(Math.max(...raws));
      expect(s.summary.alerts).toBe(s.alerts.length);
      expect(s.summary.has_rr).toBe(s.hrv.length > 0);
      const zoneTotal = s.zone_secs.reduce((x, y) => x + y, 0);
      expect(Math.abs(zoneTotal - s.samples.length)).toBeLessThan(s.samples.length * 0.05);
      for (const al of s.alerts) {
        if (al.kind === "high") expect(al.bpm).toBeGreaterThanOrEqual(al.threshold);
        else expect(al.bpm).toBeLessThanOrEqual(al.threshold);
      }
    }
    // Newest first.
    expect(a.map((s) => s.summary.start_ms)).toEqual([...a.map((s) => s.summary.start_ms)].sort((x, y) => y - x));
  });

  it("can be listed, opened, exported and deleted through the mock backend", async () => {
    mock = createMockBackend({ scenario: "idle" });
    let changed = 0;
    await mock.on("sessions", () => changed++);
    const list = await mock.listSessions();
    expect(list.length).toBe(4);
    const full = await mock.getSession(list[0].id);
    expect(full.samples.length).toBe(list[0].samples);
    // In the browser the CSV is offered as a download; jsdom can't navigate, so catch the click.
    const clicks: string[] = [];
    vi.spyOn(HTMLAnchorElement.prototype, "click").mockImplementation(function (this: HTMLAnchorElement) {
      clicks.push(this.download);
    });
    const name = await mock.exportSession(list[0].id);
    expect(name).toMatch(/^kittytooth-\d{4}-\d{2}-\d{2}-\d{4}\.csv$/);
    if (typeof URL.createObjectURL === "function") expect(clicks).toEqual([name]);
    const after = await mock.deleteSession(list[0].id);
    expect(after.map((s) => s.id)).not.toContain(list[0].id);
    expect(changed).toBe(1);
    await expect(mock.getSession(list[0].id)).rejects.toMatchObject({ title: "Session not found" });
  });
});
