/// <reference types="vitest/config" />
import { fileURLToPath, URL } from "node:url";

import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite";

// The Tauri CLI sets TAURI_ENV_* variables when it runs `pnpm dev` or
// `pnpm build`; see https://v2.tauri.app/start/frontend/vite/.
const tauriPlatform = process.env.TAURI_ENV_PLATFORM;
const tauriDebug = Boolean(process.env.TAURI_ENV_DEBUG);

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: {
      "@": fileURLToPath(new URL("./src", import.meta.url)),
      // Logo masters, shared with the README and the installer.
      "@brand": fileURLToPath(new URL("../../assets/brand", import.meta.url)),
    },
  },
  // Keep the Rust compiler output visible in `tauri dev`.
  clearScreen: false,
  server: {
    // tauri.conf.json `build.devUrl` points to this port.
    port: 1420,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    // WebView2 is evergreen Chromium; the browser target applies to the
    // Playwright and preview builds outside Tauri.
    target: tauriPlatform === "windows" ? "chrome130" : "baseline-widely-available",
    minify: tauriDebug ? false : "oxc",
    sourcemap: tauriDebug,
  },
  test: {
    environment: "happy-dom",
    include: ["src/**/*.test.{ts,tsx}"],
    restoreMocks: true,
    setupFiles: ["src/test/setup.ts"],
  },
});
