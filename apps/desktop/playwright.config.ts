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
  forbidOnly: ci,
  retries: ci ? 1 : 0,
  reporter: ci ? [["github"], ["html", { open: "never" }]] : "list",
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
