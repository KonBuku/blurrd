import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { openFresh } from "./preview-helpers";

/** Runs a command of the palette by typing its label. */
async function runCommand(page: Page, label: string) {
  await page.keyboard.press("Control+K");
  await expect(page.getByRole("combobox", { name: "Search commands" })).toBeFocused();
  await page.keyboard.type(label);
  await page.getByRole("option", { name: label }).click();
}

/** Axe violations inside the open dialogs. */
async function dialogViolations(page: Page) {
  const results = await new AxeBuilder({ page })
    .include('[role="dialog"]')
    .withTags(["wcag2a", "wcag2aa", "wcag21aa", "wcag22aa"])
    .analyze();
  return results.violations;
}

test.describe("about and licences", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page);
  });

  test("Settings > About shows the build, the links and the trademark statements", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "About" }).click();
    const about = page.getByRole("heading", { name: "About", level: 1 });
    await expect(about).toBeVisible();
    await expect(
      page.getByText(/not affiliated with, endorsed, sponsored or approved by Riot/),
    ).toBeVisible();
    await expect(page.getByText(/^\d+\.\d+\.\d+$/)).toBeVisible();
    await page.getByRole("button", { name: "Report an issue" }).click();
    await page.getByRole("button", { name: "Source code" }).click();
    await expect
      .poll(() => page.evaluate(() => window.blurrdMock?.opened))
      .toEqual([
        "https://github.com/KonBuku/blurrd/issues/new/choose",
        expect.stringMatching(/releases\/download\/deps-ffmpeg-.+-source\.zip$/),
      ]);
  });

  test("the licences view lists, searches and opens every kind of component", async ({ page }) => {
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "About" }).click();
    const opened = Date.now();
    await page.getByRole("button", { name: "Open-source licenses" }).click();
    const dialog = page.getByRole("dialog", { name: "Open-source licenses" });
    await Promise.all(
      ["Bundled programs", "Rust crates", "npm packages"].map((section) =>
        expect(dialog.getByRole("region", { name: section })).toBeVisible(),
      ),
    );
    const loadMs = Date.now() - opened;
    const rows = await dialog.locator("li > button[aria-expanded]").count();
    test.info().annotations.push({ type: "licences", description: `${rows} rows in ${loadMs} ms` });
    expect(rows).toBeGreaterThan(300);

    await dialog
      .getByRole("searchbox", { name: "Search the components" })
      .fill("tauri-plugin-opener");
    const row = dialog.getByRole("button", { name: /^tauri-plugin-opener/ });
    await expect(row).toHaveCount(1);
    await row.click();
    await expect(dialog.locator("pre").first()).toContainText(/License|LICENSE/);

    await page.keyboard.press("Escape");
    await expect(dialog).toBeHidden();
    // Focus returns to the button that opened the dialog.
    await expect(page.getByRole("button", { name: "Open-source licenses" })).toBeFocused();
  });

  test("the command palette opens About and the licences", async ({ page }) => {
    await runCommand(page, "About blurrd");
    const about = page.getByRole("dialog", { name: "blurrd" });
    await expect(about).toBeVisible();
    await expect(about.getByText(/VALORANT and Riot Games are trademarks/)).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(about).toBeHidden();

    await runCommand(page, "Open-source licenses");
    await expect(page.getByRole("dialog", { name: "Open-source licenses" })).toBeVisible();
  });

  for (const theme of ["light", "dark"] as const) {
    test(`About and the licences have no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme });
      await runCommand(page, "About blurrd");
      const about = page.getByRole("dialog", { name: "blurrd" });
      await expect(about.getByRole("button", { name: "Source code" })).toBeVisible();
      expect(await dialogViolations(page)).toEqual([]);

      await about.getByRole("button", { name: "Open-source licenses" }).click();
      const licenses = page.getByRole("dialog", { name: "Open-source licenses" });
      await licenses.getByRole("searchbox", { name: "Search the components" }).fill("ffmpeg");
      await licenses.getByRole("button", { name: /^FFmpeg/ }).click();
      expect(await dialogViolations(page)).toEqual([]);
    });
  }
});
