import { expect, test } from "@playwright/test";

import { INDEX_RECT, PLATES, readIndex } from "../src/ipc/mock/fixture-media";
import { requireFixtureMedia } from "./fixtures";
import { canvasFrame, canvasPixels, openFresh, openRecording } from "./preview-helpers";

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

test.describe("preview", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
  });

  test("shows the selected recording blurred, before and split", async ({ page }) => {
    await openRecording(page, "D:\\Recordings\\match_ascent_01.mp4");
    expect(await canvasFrame(page)).toBe(0);
    // Seek to a frame with kill-feed plates through the controller's video.
    await page.evaluate(() => {
      const video = document.querySelector("video");
      if (video) video.currentTime = 100 / 60 + 1 / 120;
    });
    await expect(page.getByTestId("preview-canvas")).toHaveAttribute("data-frame", "100");
    const plate = PLATES[0]?.rect ?? [0, 0, 1, 1];
    const after = await canvasPixels(page, plate);
    await page.getByRole("radio", { name: "Before" }).click();
    await expect
      .poll(async () => difference(await canvasPixels(page, plate), after))
      .toBeGreaterThan(8);
    const before = await canvasPixels(page, plate);
    // Outside every box the frame is untouched (the frame index code).
    const index = await canvasPixels(page, [0, 0, INDEX_RECT[2], INDEX_RECT[3]]);
    expect(readIndex(index, INDEX_RECT[2])).toBe(100);
    // Split at 50 %: the plate (right half) shows After; at 100 % Before.
    await page.getByRole("radio", { name: "Split" }).click();
    const divider = page.getByRole("slider", { name: "Split position" });
    await expect(divider).toHaveAttribute("aria-valuenow", "50");
    await expect.poll(async () => difference(await canvasPixels(page, plate), after)).toBe(0);
    await divider.focus();
    await page.keyboard.press("End");
    await expect(divider).toHaveAttribute("aria-valuenow", "100");
    await expect.poll(async () => difference(await canvasPixels(page, plate), before)).toBe(0);
  });

  test("draws the backend's own blur of a paused frame with Exact", async ({ page }) => {
    await openRecording(page, "D:\\Recordings\\match_ascent_01.mp4");
    await page.evaluate(() => {
      const video = document.querySelector("video");
      if (video) video.currentTime = 37 / 60 + 1 / 120;
    });
    const canvas = page.getByTestId("preview-canvas");
    await expect(canvas).toHaveAttribute("data-frame", "37");
    await page.getByRole("button", { name: "Exact" }).click();
    await expect(page.getByRole("button", { name: "Exact" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    // The backend decoded the same frame, and its patch sits on the code
    // (the mock draws the code inverted, so the canvas reads ~37).
    await expect(canvas).toHaveAttribute("data-exact-frame", "37");
    const read = () =>
      canvasPixels(page, [0, 0, INDEX_RECT[2], INDEX_RECT[3]]).then((px) =>
        readIndex(px, INDEX_RECT[2]),
      );
    await expect.poll(read).toBe(~37 & 0x3ff);
    await page.getByRole("button", { name: "Exact" }).click();
    await expect.poll(read).toBe(37);
  });

  test("plays an HEVC source the browser cannot decode from its proxy", async ({ page }) => {
    const hevc = await page.evaluate(() =>
      document.createElement("video").canPlayType('video/mp4; codecs="hvc1.1.6.L120.90"'),
    );
    await openRecording(page, "D:\\Recordings\\match_hevc_05.mp4");
    if (hevc === "") {
      await expect(page.getByText("Proxy", { exact: true })).toBeVisible();
      await expect(page.locator("video")).toHaveAttribute("src", /proxy/);
    }
    // Either way the frame on the canvas is the source frame 0, at source size.
    const index = await canvasPixels(page, [0, 0, INDEX_RECT[2], INDEX_RECT[3]]);
    expect(readIndex(index, INDEX_RECT[2])).toBe(0);
  });
});
