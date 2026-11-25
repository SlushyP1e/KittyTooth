import { readFileSync } from "node:fs";
import { test, type Page } from "@playwright/test";

// Generates docs/screenshots from the mock backend: `npm run screenshots`.
const OUT = "docs/screenshots";
test.use({ deviceScaleFactor: 2 });

async function setTheme(page: Page, theme: "dark" | "light") {
  await page.goto("/#/settings");
  await page.getByRole("radiogroup", { name: "Theme" }).getByRole("radio", { name: theme }).click();
  await page.waitForTimeout(300);
}

const VIEWS = [
  ["live", "/#/live", 2600],
  ["devices", "/#/devices", 1800],
  ["osc", "/#/osc", 1800],
  ["settings", "/#/settings", 600],
] as const;

for (const theme of ["dark", "light"] as const) {
  for (const width of [1280, 900] as const) {
    test(`screens ${theme} ${width}`, async ({ page }) => {
      await page.setViewportSize({ width, height: 820 });
      await setTheme(page, theme);
      for (const [name, url, wait] of VIEWS) {
        await page.goto(url);
        await page.waitForTimeout(wait);
        const suffix = `${theme === "light" ? "-light" : ""}${width === 900 ? "-900" : ""}`;
        await page.screenshot({ path: `${OUT}/${name}${suffix}.png` });
      }
    });
  }
}

test("screenshot osc log", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/#/osc");
  await page.getByRole("button", { name: "Send test sweep" }).click();
  await page.waitForTimeout(2000);
  await page.getByRole("log", { name: "OSC messages sent" }).scrollIntoViewIfNeeded();
  await page.screenshot({ path: `${OUT}/osc-log.png` });
});

test("screenshot error", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 820 });
  await page.goto("/?mock=error#/live");
  await page.waitForTimeout(800);
  await page.screenshot({ path: `${OUT}/error.png` });
});

test("screenshot mini", async ({ page }) => {
  await page.setViewportSize({ width: 232, height: 112 });
  await page.goto("/#/mini");
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/mini.png`, omitBackground: true });
});

// The OBS overlay is served by the Rust core; render the same file with a stubbed event stream.
for (const pill of [false, true]) {
  test(`screenshot overlay${pill ? " pill" : ""}`, async ({ page }) => {
    const core = "crates/kittytooth-core/src";
    await page.route("http://overlay.test/**", (route) => {
      const path = new URL(route.request().url()).pathname;
      if (path === "/overlay") return route.fulfill({ contentType: "text/html", body: readFileSync(`${core}/overlay.html`) });
      const fonts: Record<string, string> = {
        "/fonts/bricolage-800.woff2": "bricolage-grotesque-latin-800-normal.woff2",
        "/fonts/jetbrains-400.woff2": "jetbrains-mono-latin-400-normal.woff2",
      };
      if (fonts[path]) return route.fulfill({ contentType: "font/woff2", body: readFileSync(`${core}/fonts/${fonts[path]}`) });
      return route.fulfill({ status: 404 });
    });
    await page.addInitScript(() => {
      class FakeES extends EventTarget {
        onerror: (() => void) | null = null;
        constructor() {
          super();
          setTimeout(() => this.dispatchEvent(new MessageEvent("state", { data: JSON.stringify({ connected: true, bpm: 128 }) })), 50);
        }
        close() {}
      }
      (window as unknown as { EventSource: unknown }).EventSource = FakeES;
    });
    await page.setViewportSize({ width: 360, height: 150 });
    await page.goto(`http://overlay.test/overlay${pill ? "?pill=1" : ""}`);
    await page.waitForTimeout(600);
    await page.screenshot({ path: `${OUT}/overlay${pill ? "-pill" : ""}.png`, omitBackground: true });
  });
}
