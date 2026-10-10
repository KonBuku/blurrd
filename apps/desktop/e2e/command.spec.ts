import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { requireFixtureMedia } from "./fixtures";
import { openFresh, openRecording } from "./preview-helpers";

/** The palette dialog. */
function palette(page: Page) {
  return page.getByRole("dialog", { name: "Command palette" });
}

/** Opens the palette with Ctrl+K and types `text` into its search field. */
async function searchPalette(page: Page, text: string) {
  await page.keyboard.press("Control+K");
  // Keys typed before the search field has the focus would reach the page.
  await expect(page.getByRole("combobox", { name: "Search commands" })).toBeFocused();
  await page.keyboard.type(text);
}

test.describe("command palette", () => {
  test.beforeEach(async ({ page }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await openFresh(page);
  });

  test("opens with Ctrl+K and the title bar button, closes with Escape and Ctrl+K", async ({
    page,
  }) => {
    const button = page.getByRole("button", { name: "Command palette" });
    await button.focus();
    await page.keyboard.press("Control+K");
    await expect(palette(page)).toBeVisible();
    await expect(page.getByRole("combobox", { name: "Search commands" })).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(palette(page)).toBeHidden();
    // Focus returns to where it was.
    await expect(button).toBeFocused();

    await button.click();
    await expect(palette(page)).toBeVisible();
    await page.keyboard.press("Control+K");
    await expect(palette(page)).toBeHidden();
  });

  test("runs a command with the keyboard", async ({ page }) => {
    await searchPalette(page, "theme dark");
    await expect(palette(page).getByRole("option", { name: /Theme: Dark/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    await page.keyboard.press("Enter");
    await expect(palette(page)).toBeHidden();
    await expect(page.locator("html")).toHaveClass(/\bdark\b/);

    await searchPalette(page, "appearance");
    await page.getByRole("option", { name: /Settings: Appearance/ }).click();
    await expect(page.getByRole("heading", { name: "Appearance" })).toBeVisible();
  });

  test("offers the add commands through the file dialog", async ({ page }) => {
    await page.evaluate(() => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = ["D:\\Recordings\\clip_01.mp4"];
    });
    await searchPalette(page, "add recordings");
    await page.keyboard.press("Enter");
    await expect(
      page
        .getByRole("listbox", { name: "Recordings" })
        .getByRole("option", { name: "clip_01.mp4" }),
    ).toBeVisible();
    const calls = await page.evaluate(() => window.blurrdMock?.calls.map((call) => call.cmd));
    expect(calls).toContain("plugin:dialog|open");
  });

  for (const theme of ["light", "dark"] as const) {
    test(`has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await searchPalette(page, "");
      const results = await new AxeBuilder({ page })
        .include('[role="dialog"]')
        .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
        .analyze();
      expect(results.violations).toEqual([]);
    });
  }
});

test.describe("command palette with a recording", () => {
  test("switches the preview mode and the box overlay", async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, "D:\\Recordings\\match_ascent_01.mp4");
    await expect(page.getByRole("radio", { name: "After" })).toBeChecked();

    await searchPalette(page, "before blurring");
    await page.keyboard.press("Enter");
    await expect(page.getByRole("radio", { name: "Before" })).toBeChecked();

    await searchPalette(page, "box overlay");
    await expect(palette(page).getByRole("option", { name: /Hide the box overlay/ })).toBeVisible();
    await page.keyboard.press("Enter");
    await searchPalette(page, "box overlay");
    await expect(palette(page).getByRole("option", { name: /Show the box overlay/ })).toBeVisible();
    await page.keyboard.press("Escape");

    // The footer's action (Analyze, Export or Cancel) is always offered.
    await searchPalette(page, "analyze");
    await expect(
      palette(page)
        .getByRole("group", { name: "Export" })
        .getByRole("option", { name: /^(Analyze|Export|Cancel)/ }),
    ).toHaveCount(1);
  });
});
