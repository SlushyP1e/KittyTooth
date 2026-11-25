import { afterEach, describe, expect, it, vi } from "vitest";
import { AppStore } from "./app.svelte";
import { createMockBackend, type MockBackend } from "../api/mock";
import type { Backend } from "../api/backend";

let mock: MockBackend | null = null;
afterEach(() => {
  mock?.dispose();
  mock = null;
  vi.useRealTimers();
});

describe("AppStore", () => {
  it("initialises from the backend", async () => {
    mock = createMockBackend({ scenario: "demo", prefill: 120 });
    const store = new AppStore();
    await store.init(mock);
    expect(store.ready).toBe(true);
    expect(store.isMock).toBe(true);
    expect(store.snapshot.state).toBe("connected");
    expect(store.snapshot.device?.name).toBe("KittyTooth Demo Strap");
    expect(store.history.length).toBe(120);
    expect(store.config.device.last_device?.id).toBe("demo:strap");
    store.destroy();
  });

  it("debounces config saves into one backend call", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle" });
    const spy = vi.spyOn(mock, "setConfig");
    const store = new AppStore();
    await store.init(mock);
    store.updateConfig((c) => (c.osc.keepalive_ms = 500));
    store.updateConfig((c) => (c.osc.max_rate = 10));
    store.updateConfig((c) => (c.ui.theme = "light"));
    expect(store.saving).toBe("pending");
    expect(store.config.ui.theme).toBe("light");
    await vi.advanceTimersByTimeAsync(400);
    expect(spy).toHaveBeenCalledTimes(1);
    const sent = spy.mock.calls[0][0];
    expect(sent.osc.keepalive_ms).toBe(500);
    expect(sent.osc.max_rate).toBe(10);
    expect(store.saving).toBe("saved");
    store.destroy();
  });

  it("uses sanitized config returned by the backend", async () => {
    mock = createMockBackend({ scenario: "idle" });
    const store = new AppStore();
    await store.init(mock);
    store.updateConfig((c) => (c.osc.max_rate = 500));
    await store.flushConfig();
    expect(store.config.osc.max_rate).toBe(20);
    store.destroy();
  });

  it("shows plain-language errors as toasts", async () => {
    mock = createMockBackend({ scenario: "idle" });
    const store = new AppStore();
    await store.init(mock);
    const failing: Backend = {
      ...mock,
      connect: () => Promise.reject({ code: "radio_off", title: "Bluetooth is turned off", message: "m", fix: "f" }),
    };
    await store.init(failing);
    const r = await store.run((a) => a.connect("x", "y"));
    expect(r).toBeUndefined();
    expect(store.toasts.at(-1)?.title).toBe("Bluetooth is turned off");
    // Non-UserError rejections never leak raw text.
    await store.run(() => Promise.reject(new Error("TypeError: at foo.js:12")));
    expect(store.toasts.at(-1)?.title).toBe("Something went wrong");
    expect(JSON.stringify(store.toasts.at(-1))).not.toContain("foo.js");
    store.destroy();
  });

  it("counts beats and appends samples from events", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "demo", prefill: 10, sampleMs: 1000 });
    const store = new AppStore();
    await store.init(mock);
    const before = store.history.length;
    await vi.advanceTimersByTimeAsync(3100);
    expect(store.beatCount).toBeGreaterThan(2);
    expect(store.beatInterval).toBeGreaterThan(250);
    expect(store.history.length).toBeGreaterThan(before);
    expect(store.historyVersion).toBe(store.history.version);
    store.destroy();
  });

  it("dismisses toasts", () => {
    vi.useFakeTimers();
    const store = new AppStore();
    store.toast({ kind: "info", title: "hi" }, 1000);
    expect(store.toasts).toHaveLength(1);
    vi.advanceTimersByTime(1001);
    expect(store.toasts).toHaveLength(0);
  });
});
