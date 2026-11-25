import { afterEach, describe, expect, it, vi } from "vitest";
import { createMockBackend, demoTargetHr, type MockBackend } from "./mock";
import { toUserError } from "./backend";
import type { DiscoveredDevice, Snapshot } from "../types";

let mock: MockBackend | null = null;
afterEach(() => {
  mock?.dispose();
  mock = null;
  vi.useRealTimers();
});

describe("mock backend", () => {
  it("follows the same demo profile as the Rust mock", () => {
    expect(demoTargetHr(30)).toBe(64);
    expect(demoTargetHr(200)).toBeGreaterThan(145);
    expect(demoTargetHr(30 + 360)).toBe(64);
    for (let t = 0; t < 360; t += 0.5) {
      const v = demoTargetHr(t);
      expect(v).toBeGreaterThanOrEqual(55);
      expect(v).toBeLessThanOrEqual(170);
    }
  });

  it("scans, connects and streams data", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle" });
    const devices: DiscoveredDevice[][] = [];
    const snaps: Snapshot[] = [];
    await mock.on("devices", (d) => devices.push(d));
    await mock.on("snapshot", (s) => snaps.push(s));
    await mock.startScan();
    await vi.advanceTimersByTimeAsync(200);
    expect(devices.at(-1)?.some((d) => d.id === "demo:strap")).toBe(true);
    expect(devices.at(-1)?.every((d) => d.has_hr_service)).toBe(true);
    await mock.connect("demo:strap", "KittyTooth Demo Strap");
    expect(snaps.at(-1)?.state).toBe("connecting");
    await vi.advanceTimersByTimeAsync(1200);
    expect(mock.peek().state).toBe("connected");
    expect(mock.peek().bpm).toBeGreaterThan(40);
    await mock.disconnect();
    expect(mock.peek().state).toBe("idle");
    const log = await mock.getOscLog();
    expect(log.at(-1)?.address).toBe("/avatar/parameters/KittyTooth/Connected");
  });

  it("starts in an error state for the error scenario", async () => {
    mock = createMockBackend({ scenario: "error" });
    const st = await mock.getState();
    expect(st.snapshot.error?.code).toBe("radio_off");
    expect(st.snapshot.adapter).toBe("off");
  });

  it("toUserError keeps UserErrors and hides everything else", () => {
    const u = { code: "c", title: "t", message: "m", fix: "f" };
    expect(toUserError(u)).toEqual(u);
    expect(toUserError("boom").code).toBe("unexpected");
    expect(toUserError(null).fix).toMatch(/diagnostics/);
  });
});
