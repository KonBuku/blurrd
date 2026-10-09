import { execFileSync } from "node:child_process";
import { join } from "node:path";

/**
 * Generates the synthetic fixture videos of the mock backend before the
 * end-to-end tests (see `scripts/make-fixture-media.mjs`). Without FFmpeg
 * the preview tests that need them fail in CI and are skipped locally
 * (`e2e/fixtures.ts`).
 */
export default function globalSetup() {
  const script = join(import.meta.dirname, "../scripts/make-fixture-media.mjs");
  try {
    execFileSync(process.execPath, [script], { stdio: "inherit" });
  } catch (error) {
    console.warn(`fixture videos not generated (${String(error)}); preview tests will skip`);
  }
}
