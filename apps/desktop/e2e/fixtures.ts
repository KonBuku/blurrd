import { existsSync } from "node:fs";
import { join } from "node:path";

import { expect, test, type Page } from "@playwright/test";

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

/** Session storage key of the mock backend's settings file. */
const MOCK_SETTINGS_KEY = "blurrd.mock.settings";

/**
 * Shows the app in the light or the dark theme, as a user who chose it.
 * A fresh install is dark whatever Windows uses, so tests that check both
 * themes pick one instead of emulating the system colour scheme. Pages
 * opened from now on start with the choice in their settings file; an
 * open page switches with the title bar's theme menu.
 */
export async function chooseTheme(page: Page, theme: "light" | "dark"): Promise<void> {
  const choice = theme === "light" ? "Light" : "Dark";
  await page.addInitScript(
    ([key, value]) => {
      const saved = sessionStorage.getItem(key);
      const file = (saved === null ? {} : JSON.parse(saved)) as Record<string, unknown>;
      const appearance = (file.appearance ?? {}) as Record<string, unknown>;
      sessionStorage.setItem(
        key,
        JSON.stringify({
          ...file,
          schema: "blurrd-settings/1",
          version: 1,
          appearance: { ...appearance, theme: value },
        }),
      );
    },
    [MOCK_SETTINGS_KEY, choice] as const,
  );
  if (!page.url().startsWith("http")) return;
  await page.getByRole("button", { name: "Theme" }).click();
  await page.getByRole("menuitemradio", { name: choice }).click();
  await expect(page.locator("html")).toHaveClass(theme === "dark" ? /\bdark\b/ : /^(?!.*\bdark\b)/);
}
