import { expect, test, type Page } from "@playwright/test";

// Smoke tests of every screen against the in-browser mock backend (simulated sensor).

function trackErrors(page: Page): string[] {
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(String(e)));
  page.on("console", (m) => {
    if (m.type() === "error") errors.push(m.text());
  });
  return errors;
}

test.beforeEach(async ({ page }) => {
  // Count heart animations so we can prove the heart pulses with beat events.
  await page.addInitScript(() => {
    const w = window as unknown as { __heartBeats: number };
    w.__heartBeats = 0;
    const orig = Element.prototype.animate;
    Element.prototype.animate = function (this: Element, ...args: Parameters<Element["animate"]>) {
      if (this.classList.contains("heart")) w.__heartBeats++;
      return orig.apply(this, args);
    };
  });
});

test("live screen shows BPM, beat-synced heart and a painted graph", async ({ page }) => {
  const errors = trackErrors(page);
  await page.goto("/#/live");
  const bpm = page.getByTestId("bpm");
  await expect(bpm).toHaveText(/^\d{2,3}$/);
  await expect(page.getByRole("status").filter({ hasText: "Connected" }).first()).toBeVisible();
  await expect(page.getByText("KittyTooth Demo Strap").first()).toBeVisible();
  // Heart pulses on beat events (several within a few seconds).
  await expect.poll(() => page.evaluate(() => (window as unknown as { __heartBeats: number }).__heartBeats), { timeout: 6000 }).toBeGreaterThan(2);
  // The canvas has drawn the accent-colored line.
  const painted = await page.evaluate(() => {
    const c = document.querySelector(".graph canvas") as HTMLCanvasElement;
    const d = c.getContext("2d")!.getImageData(0, 0, c.width, c.height).data;
    let n = 0;
    for (let i = 0; i < d.length; i += 4) if (d[i] > 200 && d[i + 1] > 130 && d[i + 1] < 210 && d[i + 2] > 150 && d[i + 3] > 200) n++;
    return n;
  });
  expect(painted).toBeGreaterThan(100);
  // Range switch.
  await page.getByRole("radio", { name: "5 min" }).click();
  await expect(page.getByRole("radio", { name: "5 min" })).toHaveAttribute("aria-checked", "true");
  for (const label of ["Session min", "Average", "Session max", "HRV (RMSSD)"]) {
    await expect(page.getByText(label)).toBeVisible();
  }
  await expect(page.getByRole("meter", { name: "Heart rate zone" })).toBeVisible();
  expect(errors).toEqual([]);
});

test("BPM text never shifts layout (tabular numerals)", async ({ page }) => {
  await page.goto("/#/live");
  const bpm = page.getByTestId("bpm");
  await expect(bpm).toHaveText(/^\d{2,3}$/);
  const fv = await bpm.evaluate((el) => getComputedStyle(el).fontVariantNumeric);
  expect(fv).toContain("tabular-nums");
});

test("keyboard navigation reaches every screen with visible focus", async ({ page }) => {
  await page.goto("/#/live");
  await expect(page.getByTestId("bpm")).toBeVisible();
  await page.keyboard.press("Tab"); // skip link
  await expect(page.getByText("Skip to content")).toBeFocused();
  await page.keyboard.press("Tab"); // Live
  await page.keyboard.press("Tab"); // Source
  await page.keyboard.press("Tab"); // Devices
  const focused = page.locator(":focus");
  await expect(focused).toHaveText(/Devices/);
  const outline = await focused.evaluate((el) => getComputedStyle(el).outlineStyle);
  expect(outline).toBe("solid");
  await page.keyboard.press("Enter");
  await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
});

test("source page switches sources and keeps the token masked", async ({ page }) => {
  const errors = trackErrors(page);
  await page.goto("/#/source");
  await expect(page.getByRole("heading", { name: "Source", exact: true })).toBeVisible();
  const list = page.getByRole("list", { name: "Heart rate sources" });
  for (const tag of ["ble", "pulsoid", "hyperate", "osc", "demo"]) {
    await expect(list.getByText(tag, { exact: true })).toBeVisible();
  }
  await page.getByRole("button", { name: "Use Pulsoid" }).click();
  await expect(page.getByRole("alert").first()).toContainText("Pulsoid token needed");
  await expect(page.getByRole("button", { name: "Get token" })).toBeVisible();
  await page.getByLabel("Access token").fill("8c4da3ce-7ed7-4a19-a1f1-058498661e45");
  await page.getByRole("button", { name: "Save token" }).click();
  await expect(page.getByLabel("Saved token")).toHaveText("••••1e45");
  await expect(page.getByText("8c4da3ce")).toHaveCount(0);
  await page.getByRole("switch", { name: "Fall back to a backup source" }).click();
  await expect(page.getByRole("radiogroup", { name: "Backup source" })).toBeVisible();
  await page.goto("/#/live");
  await expect(page.getByTestId("bpm")).toHaveText(/^\d{2,3}$/, { timeout: 5000 });
  await expect(page.getByTitle("Source")).toHaveText("pulsoid");
  await expect(page.getByText("not available from this source").first()).toBeVisible();
  expect(errors).toEqual([]);
});

test("devices screen scans, lists and connects", async ({ page }) => {
  const errors = trackErrors(page);
  await page.goto("/#/devices");
  await expect(page.getByRole("heading", { name: "Devices" })).toBeVisible();
  const list = page.getByRole("list", { name: "Nearby devices" });
  await expect(list.getByText("Polar H10 7E21A3")).toBeVisible();
  await expect(list.getByText(/dBm/).first()).toBeVisible();
  await expect(list.getByText("MX Master 3S")).toHaveCount(0);
  await page.getByRole("switch", { name: "Show all devices" }).click();
  await expect(list.getByText("MX Master 3S")).toBeVisible();
  await page.getByRole("button", { name: "Connect to Polar H10 7E21A3" }).click();
  await expect(page.getByText("Polar H10 7E21A3").first()).toBeVisible();
  await page.getByRole("button", { name: "Forget" }).click();
  await expect(page.getByText("No sensor remembered yet")).toBeVisible();
  expect(errors).toEqual([]);
});

test("OSC screen: targets, validation, test sweep and live log", async ({ page }) => {
  const errors = trackErrors(page);
  await page.goto("/#/osc");
  await expect(page.getByRole("heading", { name: "OSC output" })).toBeVisible();
  await expect(page.getByText(/VRChat found via OSCQuery/)).toBeVisible();
  const card = page.getByRole("article", { name: "OSC target VRChat" });
  await card.getByRole("spinbutton", { name: "Port" }).fill("70000");
  await expect(card.getByText("Port must be 1-65535")).toBeVisible();
  await card.getByRole("spinbutton", { name: "Port" }).fill("9000");
  await expect(card.getByText("Port must be 1-65535")).toHaveCount(0);
  await card.getByRole("radiogroup", { name: "Parameter preset" }).getByRole("radio", { name: "Custom" }).click();
  await card.getByRole("button", { name: "Add parameter" }).click();
  await expect(card.getByRole("textbox", { name: "OSC address" })).toHaveValue("KittyTooth/Custom1");
  await page.getByRole("button", { name: "Add target" }).click();
  await expect(page.getByRole("article")).toHaveCount(2);
  await page.getByRole("button", { name: "Send test sweep" }).click();
  const log = page.getByRole("log", { name: "OSC messages sent" });
  await expect(log.getByText("test", { exact: true }).first()).toBeVisible({ timeout: 5000 });
  await page.getByRole("button", { name: "Pause" }).click();
  await expect(page.getByRole("button", { name: "Resume" })).toBeVisible();
  await page.getByRole("switch", { name: "VRChat chatbox" }).click();
  await expect(page.getByLabel("Chatbox preview")).toHaveText(/\d+ bpm/);
  expect(errors).toEqual([]);
});

test("settings: theme, accent, diagnostics", async ({ page }) => {
  const errors = trackErrors(page);
  await page.goto("/#/settings");
  await page.getByRole("radiogroup", { name: "Theme" }).getByRole("radio", { name: "Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.getByRole("radio", { name: "Accent sky" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-accent", "sky");
  await page.getByRole("radiogroup", { name: "Smoothing" }).getByRole("radio", { name: "Medium" }).click();
  await expect(page.getByText("All changes saved")).toBeVisible();
  await page.getByRole("button", { name: "Copy diagnostics" }).click();
  await expect(page.getByText("Diagnostics copied")).toBeVisible();
  await expect(page.getByText("http://127.0.0.1:9123/overlay")).toBeVisible();
  expect(errors).toEqual([]);
});

test("errors are shown in plain language with a fix", async ({ page }) => {
  await page.goto("/?mock=error#/live");
  const alert = page.getByRole("alert").first();
  await expect(alert).toContainText("Bluetooth is turned off");
  await expect(alert).toContainText("Try this:");
  await expect(alert).not.toContainText(/at .*\.(js|rs):\d+/);
  await expect(page.getByTestId("bpm")).toHaveText("--");
});

test("empty state offers the demo strap", async ({ page }) => {
  await page.goto("/?mock=idle#/live");
  await expect(page.getByText("No sensor connected yet")).toBeVisible();
  await page.getByRole("button", { name: "Try the demo strap" }).click();
  await expect(page.getByTestId("bpm")).toHaveText(/^\d{2,3}$/, { timeout: 5000 });
});

test("mini window shows BPM", async ({ page }) => {
  await page.setViewportSize({ width: 232, height: 112 });
  await page.goto("/#/mini");
  await expect(page.getByTestId("mini-bpm")).toHaveText(/^\d{2,3}$/);
});

test("respects reduced motion", async ({ browser }) => {
  const ctx = await browser.newContext({ reducedMotion: "reduce" });
  const page = await ctx.newPage();
  await page.goto("http://localhost:1420/#/live");
  await expect(page.getByTestId("bpm")).toHaveText(/^\d{2,3}$/);
  const dur = await page.evaluate(() => {
    const el = document.querySelector(".page") ?? document.body;
    return getComputedStyle(el).animationDuration;
  });
  expect(["0.001ms", "1e-06s", "0s"]).toContain(dur);
  await ctx.close();
});
