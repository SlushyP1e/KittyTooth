import { test } from "@playwright/test";

// Generates the README screenshots from the mock backend: `npm run screenshots`.
const OUT = "docs/screenshots";

test.use({ viewport: { width: 1180, height: 820 }, deviceScaleFactor: 2 });

for (const [name, url, wait] of [
  ["live", "/#/live", 2600],
  ["devices", "/#/devices", 1800],
  ["osc", "/#/osc", 1800],
  ["settings", "/#/settings", 800],
  ["error", "/?mock=error#/live", 800],
] as const) {
  test(`screenshot ${name}`, async ({ page }) => {
    await page.goto(url);
    await page.waitForTimeout(wait);
    await page.screenshot({ path: `${OUT}/${name}.png` });
  });
}

test("screenshot light theme", async ({ page }) => {
  await page.goto("/#/settings");
  await page.getByRole("radiogroup", { name: "Theme" }).getByRole("radio", { name: "Light" }).click();
  await page.goto("/#/live");
  await page.waitForTimeout(2400);
  await page.screenshot({ path: `${OUT}/live-light.png` });
});

test("screenshot mini", async ({ page }) => {
  await page.setViewportSize({ width: 232, height: 112 });
  await page.goto("/#/mini");
  await page.waitForTimeout(1500);
  await page.screenshot({ path: `${OUT}/mini.png`, omitBackground: true });
});
