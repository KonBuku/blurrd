import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { chooseTheme } from "./fixtures";

/** Opens the app with no saved settings and opens Settings › Appearance. */
async function openAppearance(page: Page) {
  await page.goto("/");
  await page.evaluate(() => {
    localStorage.clear();
    sessionStorage.clear();
  });
  await page.reload();
  await page.getByRole("button", { name: "Settings" }).click();
  await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
}

/** Background colour of the window shell, as the browser computes it. */
function shellColor(page: Page) {
  return page.evaluate(() => getComputedStyle(document.body).backgroundColor);
}

const LIGHT_SHELL = "rgb(236, 236, 238)";
const DARK_SHELL = "rgb(43, 43, 43)";

test.describe("settings and theming", () => {
  test("starts dark on a light system and follows it in System mode", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await openAppearance(page);
    const theme = page.getByRole("radiogroup", { name: "Theme" });
    await expect(theme.getByRole("radio", { name: "Dark" })).toBeChecked();
    await expect(page.locator("html")).toHaveClass(/\bdark\b/);
    expect(await shellColor(page)).toBe(DARK_SHELL);

    await theme.getByRole("radio", { name: "System" }).click();
    await expect(page.locator("html")).not.toHaveClass(/\bdark\b/);
    expect(await shellColor(page)).toBe(LIGHT_SHELL);

    await page.emulateMedia({ colorScheme: "dark" });
    await expect(page.locator("html")).toHaveClass(/\bdark\b/);
    expect(await shellColor(page)).toBe(DARK_SHELL);
  });

  test("switches the theme and keeps it after a restart", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await openAppearance(page);
    const theme = page.getByRole("radiogroup", { name: "Theme" });

    await theme.getByRole("radio", { name: "Light" }).click();
    await expect(page.locator("html")).not.toHaveClass(/\bdark\b/);
    expect(await shellColor(page)).toBe(LIGHT_SHELL);
    const saved = await page.evaluate(() => window.blurrdMock?.settingsFile());
    expect(saved).toMatchObject({ schema: "blurrd-settings/1", appearance: { theme: "Light" } });

    // The settings file is the source of truth: drop the first-paint cache.
    // The stored choice wins over the dark default.
    await page.evaluate(() => localStorage.clear());
    await page.reload();
    await expect(page.locator("html")).not.toHaveClass(/\bdark\b/);

    await page.getByRole("button", { name: "Settings" }).click();
    await page
      .getByRole("radiogroup", { name: "Theme" })
      .getByRole("radio", { name: "Dark" })
      .click();
    await expect(page.locator("html")).toHaveClass(/\bdark\b/);
    expect(await shellColor(page)).toBe(DARK_SHELL);
  });

  test("switches the theme from the title bar menu", async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await openAppearance(page);
    await page.getByRole("button", { name: "Theme" }).click();
    await page.getByRole("menuitemradio", { name: "Light" }).click();
    await expect(page.locator("html")).not.toHaveClass(/\bdark\b/);
    await expect(
      page.getByRole("radiogroup", { name: "Theme" }).getByRole("radio", { name: "Light" }),
    ).toBeChecked();
  });

  test("scales the interface and reduces motion", async ({ page }) => {
    await openAppearance(page);
    const fontSize = () =>
      page.getByRole("heading", { name: "Appearance" }).evaluate((element) => {
        return getComputedStyle(element).fontSize;
      });
    expect(await fontSize()).toBe("18px");

    await page.getByRole("combobox", { name: "Interface size" }).click();
    await page.getByRole("option", { name: "16 px" }).click();
    expect(await fontSize()).toBe("20px");

    const motion = page.getByRole("radiogroup", { name: "Reduce motion" });
    await motion.getByRole("radio", { name: "On" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-motion", "reduced");
    await motion.getByRole("radio", { name: "Off" }).click();
    await expect(page.locator("html")).toHaveAttribute("data-motion", "full");
  });

  test("closes with Escape and returns focus to the settings button", async ({ page }) => {
    await openAppearance(page);
    await expect(page.getByRole("button", { name: "Appearance" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("region", { name: "Preview" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Settings" })).toBeFocused();
  });

  for (const size of [
    { width: 1024, height: 680 },
    { width: 2560, height: 1440 },
  ]) {
    test(`fits ${size.width}x${size.height}`, async ({ page }) => {
      await page.setViewportSize(size);
      await openAppearance(page);
      const overflow = await page.evaluate(
        () => document.documentElement.scrollWidth - window.innerWidth,
      );
      expect(overflow).toBe(0);
      const row = await page.locator('[data-slot="settings-row"]').first().boundingBox();
      expect(row?.width).toBeLessThanOrEqual(896);
    });
  }
});

test.describe("settings accessibility smoke", () => {
  for (const theme of ["light", "dark"] as const) {
    for (const page of ["Appearance", "Shortcuts"] as const) {
      test(`${page} has no axe violations (${theme})`, async ({ page: browser }) => {
        await browser.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
        await chooseTheme(browser, theme);
        await openAppearance(browser);
        await browser.getByRole("button", { name: page }).click();
        await expect(browser.getByRole("heading", { name: page })).toBeVisible();
        const results = await new AxeBuilder({ page: browser }).analyze();
        expect(results.violations).toEqual([]);
      });
    }
  }
});
