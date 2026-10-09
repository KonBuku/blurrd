import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import type { Override } from "../src/bindings/Override";
import type { RectI } from "../src/bindings/RectI";
import { PLATES } from "../src/ipc/mock/fixture-media";
import { requireFixtureMedia } from "./fixtures";
import { canvasPixels, openFresh, openRecording, seekFrame } from "./preview-helpers";

const PATH = "D:\\Recordings\\match_ascent_01.mp4";

/** The box drawn in these tests: over the frame index code, which blurs visibly. */
const DRAWN: RectI = [8, 8, 128, 20];

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

/** The edits the mock backend has stored for the recording. */
function storedEdits(page: Page): Promise<readonly Override[]> {
  return page.evaluate((path) => window.blurrdMock?.projects.state(path).set.overrides ?? [], PATH);
}

/** The page point of source pixel `(x, y)` on the preview canvas. */
async function pagePoint(page: Page, x: number, y: number) {
  const canvas = await page.getByTestId("preview-canvas").boundingBox();
  if (!canvas) throw new Error("canvas not laid out");
  const scale = canvas.width / 640;
  return { x: canvas.x + x * scale, y: canvas.y + y * scale, scale };
}

/** Drags the mouse between two source points in steps. */
async function drag(page: Page, from: [number, number], to: [number, number]) {
  const a = await pagePoint(page, ...from);
  const b = await pagePoint(page, ...to);
  await page.mouse.move(a.x, a.y);
  await page.mouse.down();
  await page.mouse.move(b.x, b.y, { steps: 6 });
  await page.mouse.up();
}

/** Waits until the edits on screen are stored. */
async function saved(page: Page) {
  await expect(page.getByTestId("save-state")).toHaveAttribute("data-status", "saved");
}

test.describe("editing", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, PATH);
    await seekFrame(page, 100);
  });

  test("draws a manual box that the preview blurs, with undo and redo", async ({ page }) => {
    const initial = await storedEdits(page);
    const unverified = page.getByTestId("timeline").locator('[data-verify="unverified"]');
    await expect(unverified).toHaveCount(1);

    await page.keyboard.press("m");
    const tool = page.getByTestId("manual-box-tool");
    await expect(tool).toContainText("Draw a box · 00:00:00.66–00:00:02.66");
    await page.keyboard.press("i");
    await seekFrame(page, 130);
    const clear = await canvasPixels(page, DRAWN);
    await page.keyboard.press("o");
    await expect(tool).toContainText("frames 100 to 130");
    await drag(page, [DRAWN[0], DRAWN[1]], [DRAWN[2], DRAWN[3]]);
    await expect(tool).toHaveCount(0);

    const box = page.locator(
      '[data-overlay="box"][aria-label="Manual box · manual box · frames 100–130"]',
    );
    await expect(box).toHaveAttribute("aria-pressed", "true");
    await expect(page.getByTestId("selection-frame")).toBeVisible();
    await expect
      .poll(async () => difference(await canvasPixels(page, DRAWN), clear))
      .toBeGreaterThan(8);
    await saved(page);
    const stored = await storedEdits(page);
    expect(stored).toHaveLength(initial.length + 1);
    expect(stored.at(-1)).toMatchObject({
      op: "Add",
      region: "Manual",
      f0: 100,
      f1: 130,
      rect: DRAWN,
    });
    // Edits mark their frames unverified (verified again before export).
    await expect(unverified).toHaveCount(2);
    await expect(
      page.getByTestId("timeline").getByText(/frames 100–130, 150–170 not verified/),
    ).toBeAttached();

    await page.keyboard.press("Control+z");
    await expect(box).toHaveCount(0);
    await expect.poll(async () => difference(await canvasPixels(page, DRAWN), clear)).toBe(0);
    await saved(page);
    expect(await storedEdits(page)).toEqual(initial);

    await page.keyboard.press("Control+Shift+z");
    await expect(box).toHaveCount(1);
    await saved(page);
    expect(await storedEdits(page)).toEqual(stored);

    // With the box selected, O ends it on the shown frame.
    await seekFrame(page, 110);
    await page.keyboard.press("o");
    await expect(
      page.locator('[data-overlay="box"][aria-label="Manual box · manual box · frames 100–110"]'),
    ).toHaveCount(1);
    await saved(page);
    expect((await storedEdits(page)).at(-1)).toMatchObject({ f0: 100, f1: 110 });
  });

  test("moves, resizes and hides detections, and shows them again", async ({ page }) => {
    const plate = PLATES[0]?.rect ?? [0, 0, 1, 1];
    const feed = page.locator('[data-overlay="box"][data-key="track:1"]');
    await feed.click();
    await expect(page.getByTestId("selection-frame")).toBeVisible();

    // Drag the bottom-right handle 6 source pixels right and 3 down.
    const handle = page.locator('[data-handle="se"]');
    const at = await handle.boundingBox();
    const { scale } = await pagePoint(page, 0, 0);
    if (!at) throw new Error("handle not laid out");
    await page.mouse.move(at.x + at.width / 2, at.y + at.height / 2);
    await page.mouse.down();
    await page.mouse.move(at.x + at.width / 2 + 6 * scale, at.y + at.height / 2 + 3 * scale, {
      steps: 5,
    });
    await expect(page.getByTestId("selection-draft")).toBeVisible();
    await page.mouse.up();
    await expect(feed).toHaveAttribute("aria-label", /killer \(moved\)/);
    await saved(page);
    let adjust = (await storedEdits(page)).at(-1);
    expect(adjust).toMatchObject({
      op: "Adjust",
      region: "KillFeed",
      anchor: plate,
      rect: [plate[0], plate[1], plate[2] + 6, plate[3] + 3],
    });

    // Alt+arrows move it a pixel; the same edit changes.
    await page.keyboard.press("Alt+ArrowLeft");
    await saved(page);
    adjust = (await storedEdits(page)).at(-1);
    expect(adjust).toMatchObject({ rect: [plate[0] - 1, plate[1], plate[2] + 5, plate[3] + 3] });

    // Delete hides the detection; the hidden box can be selected and shown again.
    await page.keyboard.press("Delete");
    await expect(feed).toHaveCount(0);
    const hidden = page.locator('[data-overlay="hidden"]');
    await expect(hidden).toHaveAttribute("aria-label", /^Kill feed · hidden box · frames 12–200$/);
    await saved(page);
    expect((await storedEdits(page)).at(-1)).toMatchObject({ op: "Suppress", anchor: plate });
    await hidden.click();
    await page.keyboard.press("Delete");
    await expect(feed).toHaveCount(1);
    await saved(page);
    expect(
      (await storedEdits(page)).some(
        (edit) => edit.op === "Suppress" && edit.region === "KillFeed" && edit.f0 === 12,
      ),
    ).toBe(false);
  });

  test("reloads the stored edits after a stale revision", async ({ page }) => {
    await page.evaluate((path) => window.blurrdMock?.projects.touch(path), PATH);
    const feed = page.locator('[data-overlay="box"][data-key="track:1"]');
    await feed.click();
    await page.keyboard.press("Delete");
    await expect(page.getByText("The edits were changed elsewhere")).toBeVisible();
    // The stored version (without the hidden box) is back, the history empty.
    await expect(feed).toHaveCount(1);
    await expect(page.getByRole("button", { name: "Undo" })).toBeDisabled();
    const revision = await page.evaluate(
      (path) => window.blurrdMock?.projects.state(path).set.revision,
      PATH,
    );
    expect(revision).toBe(4);
  });

  test("keeps user boxes through a re-analysis and shows unmatched edits inactive", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Draw a manual box" }).click();
    await drag(page, [200, 200], [260, 230]);
    const manual = page.locator(
      '[data-overlay="box"][aria-label^="Manual box · manual box · frames 40–160"]',
    );
    await expect(manual).toHaveCount(1);
    await page.locator('[data-overlay="box"][data-key="track:1"]').click();
    await page.keyboard.press("Alt+ArrowDown");
    await saved(page);

    // A re-analysis finds the kill feed 8 pixels lower; the next save
    // meets the new revision and reloads.
    await page.evaluate((path) => window.blurrdMock?.projects.reanalyze(path, 8), PATH);
    await manual.click();
    await page.keyboard.press("Alt+ArrowRight");
    await expect(page.getByText("The edits were changed elsewhere")).toBeVisible();
    await expect(manual).toHaveCount(1);
    await expect(
      page.locator(
        '[data-overlay="unbound"][aria-label^="Kill feed · moved box no longer matches a detection (inactive)"]',
      ),
    ).toHaveCount(1);
  });

  test("passes axe while drawing and with a selection", async ({ page }) => {
    await page.locator('[data-overlay="box"][data-key="track:1"]').click();
    // Away from the box, so that no tooltip takes the Escape presses.
    await page.mouse.move(0, 0);
    await page.keyboard.press("m");
    await expect(page.getByTestId("manual-box-tool")).toBeVisible();
    const results = await new AxeBuilder({ page }).analyze();
    expect(results.violations).toEqual([]);
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("manual-box-tool")).toHaveCount(0);
    await page.keyboard.press("Escape");
    await expect(page.getByTestId("selection-frame")).toHaveCount(0);
  });
});
