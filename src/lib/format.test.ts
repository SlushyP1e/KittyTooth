import { describe, expect, it } from "vitest";
import {
  connectionStatus,
  formatBpm,
  formatClock,
  formatDuration,
  formatMs,
  isValidHost,
  isValidPort,
  normalizeOscAddress,
  renderChatbox,
  rssiBars,
  rssiLabel,
  timeAgo,
  zoneName,
} from "./format";
import { emptySnapshot } from "./defaults";

describe("formatting", () => {
  it("formats bpm and ms with placeholders", () => {
    expect(formatBpm(72.4)).toBe("72");
    expect(formatBpm(72.5)).toBe("73");
    expect(formatBpm(null)).toBe("--");
    expect(formatBpm(undefined)).toBe("--");
    expect(formatBpm(NaN)).toBe("--");
    expect(formatMs(41.6)).toBe("42");
    expect(formatMs(null)).toBe("--");
  });

  it("formats relative time", () => {
    const now = 1_000_000;
    expect(timeAgo(now, now)).toBe("just now");
    expect(timeAgo(now - 5_000, now)).toBe("5 s ago");
    expect(timeAgo(now - 3 * 60_000, now)).toBe("3 min ago");
    expect(timeAgo(now - 2 * 3_600_000, now)).toBe("2 h ago");
    expect(timeAgo(now + 5000, now)).toBe("just now");
  });

  it("maps RSSI to bars", () => {
    expect(rssiBars(-50)).toBe(4);
    expect(rssiBars(-65)).toBe(3);
    expect(rssiBars(-75)).toBe(2);
    expect(rssiBars(-85)).toBe(1);
    expect(rssiBars(-99)).toBe(0);
    expect(rssiBars(null)).toBe(0);
    expect(rssiLabel(-50)).toBe("Excellent");
  });

  it("names and colors zones, clamped", () => {
    expect(zoneName(0)).toBe("Rest");
    expect(zoneName(3)).toBe("Aerobic");
    expect(zoneName(9)).toBe("Max");
    expect(zoneName(-2)).toBe("Rest");
  });

  it("formats durations and clock times", () => {
    expect(formatDuration(65)).toBe("1:05");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatClock(new Date(2026, 0, 1, 9, 5, 7, 42).getTime())).toBe("09:05:07.042");
  });
});

describe("validation", () => {
  it("normalizes OSC addresses like the Rust side", () => {
    expect(normalizeOscAddress("HR")).toBe("/avatar/parameters/HR");
    expect(normalizeOscAddress(" Foo/Bar ")).toBe("/avatar/parameters/Foo/Bar");
    expect(normalizeOscAddress("/custom/addr")).toBe("/custom/addr");
    for (const bad of ["", "  ", "has space", "/a//b", "/a/", "a#b", "x*", "a,b", "q?", "[x]", "{y}"]) {
      expect(normalizeOscAddress(bad), bad).toBeNull();
    }
  });

  it("validates hosts and ports", () => {
    expect(isValidHost("127.0.0.1")).toBe(true);
    expect(isValidHost("192.168.1.255")).toBe(true);
    expect(isValidHost("256.1.1.1")).toBe(false);
    expect(isValidHost("localhost")).toBe(true);
    expect(isValidHost("my-pc.local")).toBe(true);
    expect(isValidHost("::1")).toBe(true);
    expect(isValidHost("bad host")).toBe(false);
    expect(isValidHost("")).toBe(false);
    expect(isValidPort(9000)).toBe(true);
    expect(isValidPort(0)).toBe(false);
    expect(isValidPort(65536)).toBe(false);
    expect(isValidPort(90.5)).toBe(false);
  });

});

describe("status and chatbox", () => {
  it("describes connection status", () => {
    const s = emptySnapshot();
    expect(connectionStatus(s)).toEqual({ label: "Not connected", tone: "off" });
    expect(connectionStatus({ ...s, state: "connected" }).tone).toBe("ok");
    expect(connectionStatus({ ...s, state: "no_data", contact: "not_detected" }).label).toBe("No skin contact");
    expect(connectionStatus({ ...s, state: "reconnecting" }).label).toBe("Reconnecting");
    expect(
      connectionStatus({ ...s, state: "reconnecting", error: { code: "x", title: "", message: "", fix: "" } }).tone,
    ).toBe("error");
    expect(connectionStatus({ ...s, test_sweep: true }).label).toBe("Test sweep");
  });

  it("renders the chatbox preview like the Rust template", () => {
    const s = { ...emptySnapshot(), bpm: 72, hrv_ms: 42.6, zone: 1, battery: 90 };
    s.stats = { min: 60, max: 120, avg: 80.4, samples: 3 };
    expect(renderChatbox("{hr} bpm {heart}", s, true)).toBe("72 bpm ❤️");
    expect(renderChatbox("{min}/{avg}/{max} hrv {hrv} z{zone} {battery}%", s)).toBe("60/80/120 hrv 43 z1 90%");
    expect(renderChatbox("x".repeat(300), s)).toHaveLength(144);
  });
});

describe("sources", () => {
  it("lists what a source can't provide", async () => {
    const { missingFeatures, SOURCE_INFO, SOURCE_ORDER } = await import("./format");
    expect(missingFeatures({ rr: true, battery: true, contact: true })).toEqual([]);
    expect(missingFeatures({ rr: false, battery: true, contact: false })).toEqual(["hrv", "skin contact"]);
    expect(SOURCE_ORDER.map((k) => SOURCE_INFO[k].tag)).toEqual(["ble", "ant", "pulsoid", "hyperate", "osc", "demo"]);
  });
});
