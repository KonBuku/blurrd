import { defineConfig, devices } from "@playwright/test";

const port = 1420;
const ci = Boolean(process.env.CI);

/**
 * End-to-end tests of the frontend in Chromium (the engine of WebView2)
 * against the mock backend (`vite --mode mock`), so they run without the
 * Tauri shell and without a GPU.
 */
export default defineConfig({
  testDir: "e2e",
  // Encodes the synthetic fixture videos of the preview tests.
  globalSetup: "./e2e/global-setup.ts",
  forbidOnly: ci,
  // Frame-rate and main-thread budgets (tagged @perf) are measured on a
  // quiet developer machine; shared CI runners are too slow and noisy for
  // them. Set BLURRD_E2E_PERF=1 to run them anyway.
  ...(ci && !process.env.BLURRD_E2E_PERF ? { grepInvert: /@perf/ } : {}),
  retries: ci ? 1 : 0,
  reporter: ci ? [["github"], ["html", { open: "never" }]] : "list",
  expect: {
    // Gallery screenshots (e2e/gallery.spec.ts-snapshots, rendered on
    // Windows): tolerate antialiasing differences between machines, not
    // layout or colour changes.
    toHaveScreenshot: { maxDiffPixelRatio: 0.01, animations: "disabled", caret: "hide" },
  },
  use: {
    baseURL: `http://localhost:${port}`,
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
  },
  projects: [{ name: "chromium", use: { ...devices["Desktop Chrome"] } }],
  webServer: {
    command: "pnpm dev:mock",
    url: `http://localhost:${port}`,
    reuseExistingServer: !ci,
  },
});
