import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import type { RectI } from "../src/bindings/RectI";
import { requireFixtureMedia } from "./fixtures";
import { canvasPixels, openFresh, openRecording, seekFrame } from "./preview-helpers";

const PATH = "D:\\Recordings\\match_ascent_01.mp4";

/** The first kill feed name plate of the fixture (shown on frames 12–200). */
const KILL_FEED_NAME: RectI = [470, 40, 540, 52];

/** Mean of the colour channels of RGBA pixels. */
function mean(pixels: readonly number[]): number {
  let sum = 0;
  let count = 0;
  for (let i = 0; i < pixels.length; i += 4) {
    for (let c = 0; c < 3; c += 1) sum += pixels[i + c] ?? 0;
    count += 3;
  }
  return sum / count;
}

/** Mean absolute difference of two RGBA pixel lists (colour channels). */
function difference(a: readonly number[], b: readonly number[]): number {
  let sum = 0;
  let count = 0;
  for (let i = 0; i < a.length; i += 4) {
    for (let c = 0; c < 3; c += 1) sum += Math.abs((a[i + c] ?? 0) - (b[i + c] ?? 0));
    count += 3;
  }
  return sum / count;
}

/** The inspector frame. */
function inspector(page: Page) {
  return page.getByRole("region", { name: "Inspector" });
}

test.describe("inspector", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, PATH);
    await seekFrame(page, 100);
  });

  test("blur tab: regions in two groups, the preview follows every change", async ({ page }) => {
    const panel = inspector(page);
    await expect(panel.getByRole("heading", { name: "HUD" })).toBeVisible();
    await expect(panel.getByRole("heading", { name: "Screens" })).toBeVisible();
    await expect(panel.getByText(/name tags above players.*out of scope/i)).toBeVisible();
    await expect(panel.getByRole("switch", { name: /^Kill feed 4 found/ })).toBeChecked();
    await expect(page.getByTestId("inspector-target")).toHaveText("match_ascent_01.mp4");

    const blurred = await canvasPixels(page, KILL_FEED_NAME);
    await panel.getByRole("switch", { name: /^Kill feed/ }).click();
    await expect
      .poll(async () => difference(await canvasPixels(page, KILL_FEED_NAME), blurred))
      .toBeGreaterThan(8);
    const clear = await canvasPixels(page, KILL_FEED_NAME);
    await panel.getByRole("switch", { name: /^Kill feed/ }).click();
    await expect
      .poll(async () => difference(await canvasPixels(page, KILL_FEED_NAME), blurred))
      .toBeLessThan(1);

    // A black solid fill turns the name black.
    await panel.getByRole("combobox", { name: "Method" }).click();
    await page.getByRole("option", { name: "Solid fill" }).click();
    await panel.getByRole("radio", { name: "Black" }).click();
    await expect.poll(async () => mean(await canvasPixels(page, KILL_FEED_NAME))).toBeLessThan(2);
    expect(mean(clear)).toBeGreaterThan(40);
  });

  test("re-analysis runs with the inspector's regions", async ({ page }) => {
    const panel = inspector(page);
    await panel.getByRole("switch", { name: /^Round banner/ }).click();
    await panel.getByRole("button", { name: "Advanced detection" }).click();
    await panel.getByRole("button", { name: "Re-analyze" }).click();
    await expect(page.getByRole("listbox", { name: "Recordings" })).toContainText("Ready");
    // The new analysis found no banner track: the region is off.
    await expect(panel.getByRole("switch", { name: /^Round banner none found/ })).not.toBeChecked();
    const regions = await page.evaluate(() => {
      const call = window.blurrdMock?.calls.find((c) => c.cmd === "start_analysis");
      return (call?.args as { regions: { enabled: string[] } } | undefined)?.regions.enabled;
    });
    expect(regions).toEqual([
      "KillFeed",
      "VoiceChat",
      "TextChat",
      "Scoreboard",
      "DeathRecap",
      "Spectating",
      "MatchEnd",
    ]);
  });

  test("export tab: output name, estimate and an export named unverified", async ({ page }) => {
    const panel = inspector(page);
    await panel.getByRole("tab", { name: "Export" }).click();
    const preview = page.getByTestId("output-preview");
    await expect(preview).toContainText("match_ascent_01_blurrd_UNVERIFIED.mp4");
    await expect(preview).toContainText(/About [\d.]+ [KM]B/);

    await panel.getByRole("radio", { name: "Remove" }).click();
    await expect(page.getByTestId("export-summary")).toContainText("no audio");
    await panel.getByRole("button", { name: "Export", exact: true }).click();
    const queue = page.getByRole("listbox", { name: "Recordings" });
    await expect(queue).toContainText("Unverified · review the report");
    await expect(panel.getByRole("button", { name: "Export again" })).toBeEnabled();
    const render = await page.evaluate(() => {
      const call = window.blurrdMock?.calls.find((c) => c.cmd === "start_render");
      return (call?.args as { settings: { audio: unknown } } | undefined)?.settings.audio;
    });
    expect(render).toBe("Remove");
    // The next export of the same name gets a number.
    await expect(preview).toContainText("match_ascent_01_blurrd_UNVERIFIED (2).mp4");
  });

  for (const theme of ["light", "dark"] as const) {
    test(`export tab has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      const panel = inspector(page);
      await panel.getByRole("tab", { name: "Export" }).click();
      await panel.getByRole("radio", { name: "Quality" }).click();
      await panel.getByRole("button", { name: "Advanced export" }).click();
      await expect(page.getByTestId("output-preview")).toContainText(".mp4");
      const results = await new AxeBuilder({ page }).include('[aria-label="Inspector"]').analyze();
      expect(results.violations).toEqual([]);
    });

    test(`blur tab has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      const panel = inspector(page);
      await panel.getByRole("switch", { name: /^Text chat/ }).focus();
      await panel.getByRole("button", { name: "Advanced detection" }).click();
      const results = await new AxeBuilder({ page }).include('[aria-label="Inspector"]').analyze();
      expect(results.violations).toEqual([]);
    });
  }
});
