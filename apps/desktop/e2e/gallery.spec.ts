import { expect, test, type Page } from "@playwright/test";

const THEMES = ["light", "dark"] as const;

/** Opens the component gallery in the given colour scheme. */
async function openGallery(page: Page, theme: (typeof THEMES)[number]) {
  await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
  await page.goto("/?gallery");
  await expect(page.getByRole("heading", { name: "Component gallery" })).toBeVisible();
  await page.evaluate(() => document.fonts.ready);
}

// Baselines are rendered on Windows (the only target, where the system UI
// font is Segoe UI); other platforms would render different glyphs, so the
// CI job `screenshots (windows)` runs these and the Linux job skips them.
test.describe("gallery screenshots", { tag: "@screenshot" }, () => {
  test.skip(process.platform !== "win32", "screenshot baselines are rendered on Windows");

  for (const theme of THEMES) {
    test(`all components (${theme})`, async ({ page }) => {
      await openGallery(page, theme);
      await expect(page).toHaveScreenshot(`gallery-${theme}.png`, { fullPage: true });
    });

    test(`window shell (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await page.setViewportSize({ width: 1024, height: 680 });
      await page.goto("/");
      await expect(page.getByRole("banner")).toBeVisible();
      await expect(page).toHaveScreenshot(`shell-${theme}.png`);
    });

    test(`menu, dialog and toast (${theme})`, async ({ page }) => {
      await openGallery(page, theme);
      const menuTrigger = page.getByRole("button", { name: "Queue item" });
      await menuTrigger.scrollIntoViewIfNeeded();
      await menuTrigger.click();
      await expect(page.getByRole("menu")).toBeVisible();
      await expect(page).toHaveScreenshot(`menu-${theme}.png`);
      await page.keyboard.press("Escape");

      await page.getByRole("button", { name: "Verify report" }).click();
      await expect(page.getByRole("dialog")).toBeVisible();
      await expect(page).toHaveScreenshot(`dialog-${theme}.png`);
      await page.keyboard.press("Escape");

      await page.getByRole("button", { name: "Show toast" }).click();
      await expect(page.getByText("Export finished")).toBeVisible();
      await expect(page).toHaveScreenshot(`toast-${theme}.png`);
    });
  }
});

test.describe("gallery accessibility", () => {
  for (const theme of THEMES) {
    test(`every control shows a visible focus ring (${theme})`, async ({ page }) => {
      await openGallery(page, theme);
      const missing: string[] = [];
      let visited = 0;

      for (let step = 0; step < 200; step += 1) {
        // Each Tab depends on the focus the previous one left behind.
        // oxlint-disable-next-line no-await-in-loop
        await page.keyboard.press("Tab");
        // oxlint-disable-next-line no-await-in-loop
        const focus = await page.evaluate(() => {
          const element = document.activeElement;
          if (!(element instanceof HTMLElement) || element === document.body) return null;
          // Stop when the focus wraps around to an element seen before.
          if (element.dataset.focusVisited !== undefined) return null;
          element.dataset.focusVisited = "";
          // A number field draws the ring on its wrapper.
          const ringed = element.closest('[data-slot="number-field"]') ?? element;
          const style = getComputedStyle(ringed);
          const name =
            element.getAttribute("aria-label") ??
            element.closest("label")?.textContent?.trim() ??
            element.textContent?.trim().slice(0, 40) ??
            element.tagName;
          return {
            name: `${element.tagName} ${name}`,
            ring: style.outlineStyle !== "none" && Number.parseFloat(style.outlineWidth) >= 2,
          };
        });
        if (focus === null) break;
        visited += 1;
        if (!focus.ring) missing.push(focus.name);
      }

      expect(visited).toBeGreaterThan(30);
      expect(missing).toEqual([]);
    });
  }

  test("logs no console errors or warnings", async ({ page }) => {
    const messages: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error" || message.type() === "warning") messages.push(message.text());
    });
    await openGallery(page, "light");
    await page.getByRole("button", { name: "Verify report" }).click();
    await expect(page.getByRole("dialog")).toBeVisible();
    expect(messages).toEqual([]);
  });
});
