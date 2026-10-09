import { existsSync } from "node:fs";
import { join } from "node:path";

import { test } from "@playwright/test";

import { FIXTURE_URLS } from "../src/ipc/mock/fixture-media";

/**
 * Skips the current tests when the fixture videos are missing (FFmpeg not
 * installed); in CI (`CI` set) a missing fixture fails instead.
 */
export function requireFixtureMedia() {
  const root = join(import.meta.dirname, "..");
  const missing = Object.values(FIXTURE_URLS).filter((url) => !existsSync(join(root, url)));
  if (missing.length === 0) return;
  if (process.env.CI) throw new Error(`fixture videos missing: ${missing.join(", ")}`);
  test.skip(true, `fixture videos missing (install FFmpeg): ${missing.join(", ")}`);
}
