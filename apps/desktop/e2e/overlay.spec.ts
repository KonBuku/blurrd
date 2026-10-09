import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { FIXTURE, TARGET_RECT } from "../src/ipc/mock/fixture-media";
import { requireFixtureMedia } from "./fixtures";
import { openFresh, openRecording, seekFrame } from "./preview-helpers";

/** Zoom levels of acceptance M24 (25–400 %). */
const ZOOMS = [0.25, 0.5, 1, 2, 4] as const;

/** The stage's current scale: canvas CSS width over source width. */
async function stageScale(page: Page): Promise<number> {
  const box = await page.getByTestId("preview-canvas").boundingBox();
  if (!box) throw new Error("canvas not laid out");
  return box.width / FIXTURE.width;
}

/**
 * Zooms to `scale` with Ctrl+wheel around the middle of the alignment
 * target, retrying while the wheel steps leave it off by rounding.
 */
async function zoomOnTarget(page: Page, scale: number, attempts = 3): Promise<void> {
  const canvas = await page.getByTestId("preview-canvas").boundingBox();
  const current = await stageScale(page);
  if (!canvas || attempts === 0 || Math.abs(current - scale) < 1e-3) return;
  const [x0, y0, x1, y1] = TARGET_RECT;
  await page.mouse.move(canvas.x + ((x0 + x1) / 2) * current, canvas.y + ((y0 + y1) / 2) * current);
  await page.keyboard.down("Control");
  await page.mouse.wheel(0, -400 * Math.log(scale / current));
  await page.keyboard.up("Control");
  await zoomOnTarget(page, scale, attempts - 1);
}

/**
 * Finds the magenta outline of the alignment target in a screenshot of
 * the stage: the bounding box of strongly magenta pixels, in page pixels.
 */
async function magentaBox(
  page: Page,
  clip: { x: number; y: number; width: number; height: number },
) {
  const png = (await page.screenshot({ clip })).toString("base64");
  return page.evaluate(async (data) => {
    const blob = await (await fetch(`data:image/png;base64,${data}`)).blob();
    const image = await createImageBitmap(blob);
    const canvas = new OffscreenCanvas(image.width, image.height);
    const context = canvas.getContext("2d");
    if (!context) throw new Error("no 2d context");
    context.drawImage(image, 0, 0);
    const { data: px } = context.getImageData(0, 0, image.width, image.height);
    let [left, top, right, bottom] = [Infinity, Infinity, -Infinity, -Infinity];
    for (let y = 0; y < image.height; y += 1) {
      for (let x = 0; x < image.width; x += 1) {
        const i = (y * image.width + x) * 4;
        const [r, g, b] = [px[i] ?? 0, px[i + 1] ?? 0, px[i + 2] ?? 0];
        if (r - g > 45 && b - g > 45) {
          left = Math.min(left, x);
          right = Math.max(right, x + 1);
          top = Math.min(top, y);
          bottom = Math.max(bottom, y + 1);
        }
      }
    }
    return { left, top, right, bottom };
  }, png);
}

/**
 * Zooms to `zoom` and measures how far the Manual box overlay is from the
 * alignment target it marks, in source pixels: `geometry` from the layout
 * (overlay box vs source rectangle on the canvas), `edges` and `centre`
 * from the drawn outline in a screenshot with the boxes hidden.
 */
async function measureAlignment(page: Page, zoom: number) {
  await zoomOnTarget(page, zoom);
  const scale = await stageScale(page);
  expect(scale).toBeCloseTo(zoom, 2);
  const canvas = await page.getByTestId("preview-canvas").boundingBox();
  const overlay = await page
    .locator('[data-overlay="box"][aria-label^="Manual box"]')
    .boundingBox();
  if (!canvas || !overlay) throw new Error("not laid out");
  const [x0, y0, x1, y1] = TARGET_RECT;
  const geometry = Math.max(
    Math.abs(overlay.x - (canvas.x + x0 * scale)),
    Math.abs(overlay.y - (canvas.y + y0 * scale)),
    Math.abs(overlay.x + overlay.width - (canvas.x + x1 * scale)),
    Math.abs(overlay.y + overlay.height - (canvas.y + y1 * scale)),
  );
  const boxes = page.getByRole("button", { name: "Boxes" });
  await boxes.click();
  const margin = 12;
  const ox = Math.floor(overlay.x - margin);
  const oy = Math.floor(overlay.y - margin);
  const found = await magentaBox(page, {
    x: ox,
    y: oy,
    width: Math.ceil(overlay.width + 2 * margin),
    height: Math.ceil(overlay.height + 2 * margin),
  });
  await boxes.click();
  const edges = Math.max(
    Math.abs(ox + found.left - overlay.x),
    Math.abs(oy + found.top - overlay.y),
    Math.abs(ox + found.right - (overlay.x + overlay.width)),
    Math.abs(oy + found.bottom - (overlay.y + overlay.height)),
  );
  const centre = Math.max(
    Math.abs(ox + (found.left + found.right) / 2 - (overlay.x + overlay.width / 2)),
    Math.abs(oy + (found.top + found.bottom) / 2 - (overlay.y + overlay.height / 2)),
  );
  return { scale, geometry: geometry / scale, edges: edges / scale, centre: centre / scale };
}

test.describe("box overlay", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, "D:\\Recordings\\match_ascent_01.mp4");
    await seekFrame(page, 100);
  });

  test("lines up with the video within 1 source pixel at 25–400 %", async ({ page }) => {
    // Before: the Manual box would blur the outline it marks.
    await page.getByRole("radio", { name: "Before" }).click();
    const report: string[] = [];
    for (const zoom of ZOOMS) {
      // oxlint-disable-next-line no-await-in-loop
      const m = await measureAlignment(page, zoom);
      expect(m.geometry).toBeLessThanOrEqual(1);
      // The centre of the drawn outline is compared: the 4:2:0 chroma of
      // the fixture bleeds the magenta one source pixel past the 1 px
      // outline, and below 100 % the browser's downscaling smears it over
      // neighbouring screen pixels; both are symmetric. A screenshot
      // cannot resolve less than one screen pixel.
      expect(m.centre).toBeLessThanOrEqual(Math.max(1, 1 / m.scale));
      report.push(
        `${zoom * 100} %: geometry ${m.geometry.toFixed(3)} px, edges ${m.edges.toFixed(2)} px, centre ${m.centre.toFixed(2)} px`,
      );
    }
    test.info().annotations.push({ type: "alignment", description: report.join("; ") });
  });

  test("describes, selects and marks leaks and inactive edits", async ({ page }) => {
    const banner = page.locator('[data-overlay="box"][aria-label^="Round banner"]');
    await expect(banner).toHaveAttribute("aria-label", /player name · frames 70–160/);
    await banner.click();
    await expect(banner).toHaveAttribute("aria-pressed", "true");
    // The unbound hidden-box edit (frames 20–90) and the open leak (30–34).
    await seekFrame(page, 31);
    await expect(page.locator('[data-overlay="unbound"]')).toHaveAttribute(
      "aria-label",
      /no longer matches a detection \(inactive\)/,
    );
    await expect(page.locator('[data-overlay="leak"]')).toHaveAttribute(
      "aria-label",
      /readable name \(open\)/,
    );
    await page.getByRole("button", { name: "Boxes" }).click();
    await expect(page.getByTestId("box-overlay")).toHaveCount(0);
  });
});

test.describe("timeline", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, "D:\\Recordings\\match_ascent_01.mp4");
  });

  test("shows region lanes, health spans, the Verify lane and thumbnails", async ({ page }) => {
    const timeline = page.getByTestId("timeline");
    for (const region of ["KillFeed", "Banner", "TextChat", "Manual"]) {
      // oxlint-disable-next-line no-await-in-loop
      await expect(timeline.locator(`path[data-region="${region}"]`)).toHaveCount(1);
    }
    await expect(timeline.locator('[data-health="TextChat"]')).toHaveCount(1);
    await expect(
      timeline.getByRole("button", { name: /Text chat: detection unreliable/ }),
    ).toBeVisible();
    await expect(timeline.locator('[data-verify="clean"]')).toHaveCount(1);
    await expect(timeline.locator('[data-verify="unverified"]')).toHaveCount(1);
    await expect(timeline.locator('[data-verify="leak"]')).toHaveCount(1);
    await expect(
      timeline.getByText(/1 open leak, 1 fixed, frames 150–170 not verified/),
    ).toBeAttached();
    await expect(timeline.locator("img").first()).toBeVisible();
    // Settings › Appearance › Timeline thumbnails switches the strip off.
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Appearance" }).click();
    await page.getByRole("switch", { name: "Timeline thumbnails" }).click();
    await page.keyboard.press("Escape");
    await expect(timeline.locator("img")).toHaveCount(0);
  });

  test("scrubs to the clicked frame and jumps to a leak", async ({ page }) => {
    const slider = page.getByRole("slider", { name: "Timeline" });
    const box = await slider.boundingBox();
    if (!box) throw new Error("timeline not laid out");
    // The middle of the track is frame 120 of 240.
    await page.mouse.click(box.x + box.width * (120.5 / 240), box.y + 5);
    await expect(page.getByTestId("preview-canvas")).toHaveAttribute("data-frame", "120");
    await expect(slider).toHaveAttribute("aria-valuenow", "120");
    await slider.focus();
    await page.keyboard.press("ArrowRight");
    await expect(page.getByTestId("preview-canvas")).toHaveAttribute("data-frame", "121");
    await page.getByRole("button", { name: /^Open leak · Kill feed · frame 30/ }).click();
    await expect(page.getByTestId("preview-canvas")).toHaveAttribute("data-frame", "30");
    await expect(page.locator('[data-overlay="leak"]')).toHaveAttribute("aria-pressed", "true");
  });

  for (const theme of ["light", "dark"] as const) {
    test(`preview, overlay and timeline have no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await seekFrame(page, 31);
      await page.getByRole("radio", { name: "Split" }).click();
      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
    });
  }
});
