import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { chooseTheme } from "./fixtures";

/** Window sizes from the minimum window to a maximized 1440p screen. */
const SIZES = [
  { width: 1024, height: 680 },
  { width: 1179, height: 760 },
  { width: 1180, height: 760 },
  { width: 1440, height: 900 },
  { width: 1920, height: 1080 },
  { width: 2560, height: 1440 },
] as const;

/** Width of the element matched by `selector`, or 0 when it is missing. */
async function widthOf(page: Page, selector: string): Promise<number> {
  const box = await page.locator(selector).boundingBox();
  return box?.width ?? 0;
}

/** Opens the app with a clean layout and waits for the first render. */
async function openApp(page: Page) {
  await page.goto("/");
  await page.evaluate(() => localStorage.clear());
  await page.reload();
  await expect(page.getByRole("region", { name: "Preview" })).toBeVisible();
}

test.describe("window layout", () => {
  for (const size of SIZES) {
    test(`fits ${size.width}x${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await openApp(page);

      // Nothing overflows the window.
      const overflow = await page.evaluate(() => ({
        x: document.documentElement.scrollWidth - window.innerWidth,
        y: document.documentElement.scrollHeight - window.innerHeight,
      }));
      expect(overflow).toEqual({ x: 0, y: 0 });

      const queue = await widthOf(page, "#queue-sidebar");
      expect(queue).toBeGreaterThanOrEqual(264);
      expect(queue).toBeLessThanOrEqual(size.width * 0.4);
      expect(await widthOf(page, '[aria-labelledby="preview-title"]')).toBeGreaterThanOrEqual(480);

      const inspector = page.getByRole("region", { name: "Inspector" });
      if (size.width < 1180) {
        await expect(inspector).toHaveCount(0);
        await page.getByRole("button", { name: "Show inspector" }).click();
        const sheet = page.getByRole("dialog", { name: "Inspector" });
        await expect(sheet).toBeVisible();
        await expect(sheet.getByRole("tab", { name: "Blur" })).toBeVisible();
        await page.keyboard.press("Escape");
        await expect(sheet).toHaveCount(0);
        await expect(page.getByRole("button", { name: "Show inspector" })).toBeFocused();
      } else {
        await expect(inspector).toBeVisible();
        const width = await widthOf(page, "#inspector");
        expect(width).toBeGreaterThanOrEqual(300);
        expect(width).toBeLessThanOrEqual(420);
      }
    });
  }

  test("drags the queue and inspector edges and keeps the widths", async ({ page }) => {
    await openApp(page);
    const drag = async (name: string, dx: number) => {
      const box = await page.getByRole("separator", { name }).boundingBox();
      if (!box) throw new Error(`${name} handle is not visible`);
      const x = box.x + box.width / 2;
      const y = box.y + box.height / 2;
      await page.mouse.move(x, y);
      await page.mouse.down();
      await page.mouse.move(x + dx, y, { steps: 5 });
      await page.mouse.up();
    };

    await drag("Resize queue", 60);
    await drag("Resize inspector", -80);
    expect(await widthOf(page, "#queue-sidebar")).toBe(340);
    expect(await widthOf(page, "#inspector")).toBe(400);

    await page.reload();
    await expect(page.getByRole("region", { name: "Preview" })).toBeVisible();
    expect(await widthOf(page, "#queue-sidebar")).toBe(340);
    expect(await widthOf(page, "#inspector")).toBe(400);
  });

  test("collapses the queue with Ctrl+B and takes it out of the tab order", async ({ page }) => {
    await openApp(page);
    await page.keyboard.press("Control+B");
    await expect(page.getByRole("button", { name: "Show queue" })).toBeVisible();
    await expect.poll(() => widthOf(page, "#queue-sidebar")).toBe(0);
    await expect(page.locator("#queue-sidebar")).toHaveAttribute("inert", "");

    await page.keyboard.press("Control+B");
    await expect.poll(() => widthOf(page, "#queue-sidebar")).toBe(280);
  });
});

test.describe("accessibility smoke", () => {
  for (const theme of ["light", "dark"] as const) {
    test(`shell has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await chooseTheme(page, theme);
      await openApp(page);
      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
    });

    test(`inspector sheet has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await chooseTheme(page, theme);
      await page.setViewportSize({ width: 1024, height: 680 });
      await openApp(page);
      await page.getByRole("button", { name: "Show inspector" }).click();
      await expect(page.getByRole("dialog", { name: "Inspector" })).toBeVisible();
      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
    });
  }
});
