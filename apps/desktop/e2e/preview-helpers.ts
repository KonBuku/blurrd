import { expect, type Page } from "@playwright/test";

import type { RectI } from "../src/bindings/RectI";

/** Opens the app with nothing saved, an instant probe and GPU check. */
export async function openFresh(page: Page) {
  await page.goto("/");
  await page.evaluate(() => {
    localStorage.clear();
    sessionStorage.clear();
    sessionStorage.setItem("blurrd.mock.capabilitiesDelayMs", "0");
  });
  await page.reload();
  await expect(page.getByRole("complementary", { name: "Queue" })).toBeVisible();
  await page.evaluate(() => {
    if (window.blurrdMock) window.blurrdMock.probeDelayMs = 0;
  });
}

/** Adds `path` through the file dialog and selects it, then waits for its first frame. */
export async function openRecording(page: Page, path: string) {
  await page.evaluate((file) => {
    if (window.blurrdMock) window.blurrdMock.dialogResult = [file];
  }, path);
  await page.getByRole("button", { name: "Add recordings" }).click();
  const name = path.slice(path.lastIndexOf("\\") + 1);
  await page.getByRole("listbox", { name: "Recordings" }).getByRole("option", { name }).click();
  await expect(page.getByTestId("preview-canvas")).toHaveAttribute("data-frame", /^\d+$/);
}

/** The presented frame index the canvas reports. */
export async function canvasFrame(page: Page): Promise<number> {
  return Number(await page.getByTestId("preview-canvas").getAttribute("data-frame"));
}

/**
 * Reads RGBA pixels of a source rectangle from the preview canvas (the
 * canvas is at source resolution, so canvas pixels are source pixels).
 */
export async function canvasPixels(page: Page, rect: RectI): Promise<number[]> {
  return page.evaluate(([x0, y0, x1, y1]) => {
    const canvas = document.querySelector<HTMLCanvasElement>('[data-testid="preview-canvas"]');
    if (!canvas) throw new Error("no preview canvas");
    const copy = document.createElement("canvas");
    copy.width = canvas.width;
    copy.height = canvas.height;
    const context = copy.getContext("2d");
    if (!context) throw new Error("no 2d context");
    context.drawImage(canvas, 0, 0);
    return Array.from(context.getImageData(x0, y0, x1 - x0, y1 - y0).data);
  }, rect);
}
