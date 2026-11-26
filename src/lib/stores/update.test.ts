import { afterEach, describe, expect, it, vi } from "vitest";
import { CHECK_INTERVAL_MS, UpdateStore } from "./update.svelte";
import { createMockBackend, MOCK_UPDATE_VERSION, type MockBackend } from "../api/mock";

let mock: MockBackend | null = null;
let store: UpdateStore | null = null;
afterEach(() => {
  store?.stop();
  store = null;
  mock?.dispose();
  mock = null;
  vi.useRealTimers();
});

describe("UpdateStore", () => {
  it("checks on launch and every 6 hours while auto-check is on", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle" });
    const spy = vi.spyOn(mock, "checkUpdate");
    store = new UpdateStore();
    let auto = true;
    await store.start(mock, () => auto, 1000);
    expect(spy).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1500);
    expect(spy).toHaveBeenCalledTimes(1);
    expect(store.status).toBe("available");
    expect(store.info?.version).toBe(MOCK_UPDATE_VERSION);
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS);
    expect(spy).toHaveBeenCalledTimes(2);
    auto = false;
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS);
    expect(spy).toHaveBeenCalledTimes(2);
  });

  it("never installs on its own", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle" });
    const install = vi.spyOn(mock, "installUpdate");
    store = new UpdateStore();
    await store.start(mock, () => true, 10);
    await vi.advanceTimersByTimeAsync(CHECK_INTERVAL_MS * 3 + 1000);
    expect(store.status).toBe("available");
    expect(install).not.toHaveBeenCalled();
  });

  it("installs with a click and reports progress", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle" });
    store = new UpdateStore();
    await store.start(mock, () => false, 10);
    const checking = store.check();
    await vi.advanceTimersByTimeAsync(500);
    await checking;
    expect(store.info?.can_install).toBe(true);
    const seen: number[] = [];
    await mock.on("update_progress", (p) => seen.push(p.downloaded));
    const done = store.install();
    expect(store.status).toBe("downloading");
    await vi.advanceTimersByTimeAsync(2000);
    expect(await done).toBe(true);
    expect(seen.length).toBeGreaterThan(1);
    expect(store.status).toBe("up_to_date");
    expect(store.info).toBeNull();
  });

  it("offers the release page instead of installing on portable copies", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle", update: "portable" });
    const install = vi.spyOn(mock, "installUpdate");
    const open = vi.spyOn(mock, "openRelease").mockResolvedValue();
    store = new UpdateStore();
    await store.start(mock, () => false, 10);
    const checking = store.check();
    await vi.advanceTimersByTimeAsync(500);
    await checking;
    expect(store.info?.install_kind).toBe("portable");
    expect(store.info?.can_install).toBe(false);
    expect(await store.install()).toBe(false);
    expect(install).not.toHaveBeenCalled();
    await store.openRelease();
    expect(open).toHaveBeenCalledWith(MOCK_UPDATE_VERSION);
  });

  it("reports up to date and quiet failures", async () => {
    vi.useFakeTimers();
    mock = createMockBackend({ scenario: "idle", update: "none" });
    store = new UpdateStore();
    await store.start(mock, () => false, 10);
    let p = store.check();
    await vi.advanceTimersByTimeAsync(500);
    await p;
    expect(store.status).toBe("up_to_date");
    expect(store.lastChecked).not.toBeNull();

    mock.dispose();
    mock = createMockBackend({ scenario: "idle", update: "error" });
    store.stop();
    await store.start(mock, () => false, 10);
    p = store.check();
    await vi.advanceTimersByTimeAsync(500);
    await p;
    expect(store.status).toBe("error");
    expect(store.error?.code).toBe("update_check_failed");
  });
});
