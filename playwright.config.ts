import { defineConfig, devices } from "@playwright/test";

const screenshots = !!process.env.SCREENSHOTS;

export default defineConfig({
  testDir: "e2e",
  testMatch: screenshots ? /screenshots\.spec\.ts/ : /smoke\.spec\.ts/,
  fullyParallel: true,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  reporter: process.env.CI ? [["list"], ["html", { open: "never" }]] : "list",
  outputDir: "test-results",
  use: {
    baseURL: "http://localhost:1420",
    trace: "off",
    viewport: { width: 1180, height: 820 },
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"], viewport: { width: 1180, height: 820 } } }],
  webServer: {
    command: "npm run build && npx vite preview --port 1420 --strictPort",
    url: "http://localhost:1420",
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
