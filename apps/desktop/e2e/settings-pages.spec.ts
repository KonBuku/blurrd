// The pages are visited one after another, so the loops await in turn.
/* oxlint-disable no-await-in-loop */
import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { chooseTheme, requireFixtureMedia } from "./fixtures";
import { openFresh, openRecording } from "./preview-helpers";

const PATH = "D:\\Recordings\\match_ascent_01.mp4";
const EXPORTS = "E:\\Exports\\blurred";

/** The new settings pages, as the navigation names them, with their headings. */
const PAGES = ["Processing", "Output defaults", "Detection", "Verification", "Logs & privacy"];

function nav(page: Page) {
  return page.getByRole("navigation", { name: "Settings pages" });
}

async function openPage(page: Page, name: string) {
  const settings = page.getByRole("button", { name: "Settings", exact: true });
  if ((await settings.getAttribute("aria-pressed")) !== "true") await settings.click();
  await nav(page).getByRole("button", { name, exact: true }).click();
  await expect(page.getByRole("heading", { level: 1, name, exact: true })).toBeVisible();
}

/** The mock backend's settings file. */
function settingsFile(page: Page) {
  return page.evaluate(() => window.blurrdMock?.settingsFile() ?? null);
}

test.describe("settings pages", () => {
  for (const theme of ["dark", "light"] as const) {
    test(`every new page renders without axe violations (${theme})`, async ({ page }) => {
      await chooseTheme(page, theme);
      await openFresh(page);
      for (const name of PAGES) {
        await openPage(page, name);
        const results = await new AxeBuilder({ page })
          .include('section[aria-label="Settings"]')
          .analyze();
        expect(results.violations, `${name}: ${JSON.stringify(results.violations)}`).toEqual([]);
      }
    });
  }

  test("fit the smallest window without horizontal scrolling", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 680 });
    await openFresh(page);
    for (const name of PAGES) {
      await openPage(page, name);
      const overflow = await page.evaluate(() => {
        const main = document.querySelector('section[aria-label="Settings"]');
        const scroller = main?.querySelector<HTMLElement>("div.overflow-y-auto[tabindex='0']");
        return scroller ? scroller.scrollWidth - scroller.clientWidth : -1;
      });
      expect(overflow, name).toBe(0);
    }
  });

  test("the GPU cards keep their text inside at 1024x680 and 14-16 px", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 680 });
    await openFresh(page);
    for (const size of [14, 16]) {
      await page.evaluate(
        (px) => document.documentElement.style.setProperty("--ui-font-size", `${px}px`),
        size,
      );
      await expect(page.getByTestId("gpu-details").first()).toContainText("NVENC");
      for (const view of ["workspace", "processing"] as const) {
        if (view === "processing") await openPage(page, "Processing");
        const outside = await page.evaluate(() =>
          [...document.querySelectorAll("[aria-busy]")].flatMap((card) => {
            const box = card.getBoundingClientRect();
            return [...card.querySelectorAll("*")]
              .filter((e) => {
                const r = e.getBoundingClientRect();
                return r.right > box.right + 0.5 || e.scrollWidth > e.clientWidth + 1;
              })
              .map((e) => e.textContent ?? "");
          }),
        );
        expect(outside, `${view} at ${size} px`).toEqual([]);
      }
      await page.getByRole("button", { name: "Close settings" }).click();
    }
  });

  test("the queue's GPU card opens the processing settings, by mouse and keyboard", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1024, height: 680 });
    await openFresh(page);
    const card = page.getByRole("button", { name: /NVENC/ });
    await expect(card).toHaveAccessibleDescription("Opens the processing settings.");
    await card.click();
    await expect(page.getByRole("heading", { level: 1, name: "Processing" })).toBeVisible();
    await page.getByRole("button", { name: "Close settings" }).click();
    await openPage(page, "Appearance");
    await page.getByRole("button", { name: "Close settings" }).click();
    await card.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { level: 1, name: "Processing" })).toBeVisible();
  });

  test("open every page at its top", async ({ page }) => {
    await page.setViewportSize({ width: 1024, height: 680 });
    await openFresh(page);
    await openPage(page, "Detection");
    const scroller = page.locator(
      'section[aria-label="Settings"] div.overflow-y-auto[tabindex="0"]',
    );
    await scroller.evaluate((element) => {
      element.scrollTop = element.scrollHeight;
    });
    await openPage(page, "Output defaults");
    expect(await scroller.evaluate((element) => element.scrollTop)).toBe(0);
  });

  test("are reachable and usable by keyboard", async ({ page }) => {
    await openFresh(page);
    await openPage(page, "Logs & privacy");
    const processing = nav(page).getByRole("button", { name: "Processing", exact: true });
    await processing.focus();
    await page.keyboard.press("Enter");
    await expect(page.getByRole("heading", { level: 1, name: "Processing" })).toBeVisible();
    const gpu = page.getByRole("switch", { name: "Use the GPU" });
    await gpu.focus();
    await page.keyboard.press("Space");
    await expect(gpu).not.toBeChecked();
    await expect(page.getByText(/Restart blurrd to apply/)).toBeVisible();
  });

  test("keep their values after a restart", async ({ page }) => {
    await openFresh(page);
    await page.evaluate((folder) => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = [folder];
    }, EXPORTS);
    await openPage(page, "Output defaults");
    await page.getByRole("button", { name: /^Change/ }).click();
    await expect(page.getByTestId("default-export-folder")).toHaveText(EXPORTS);
    await page.getByLabel("File name").fill("{name}_clean");
    await page.getByLabel("File name").press("Enter");

    await openPage(page, "Detection");
    await page.getByRole("switch", { name: /Voice chat/ }).click();
    await openPage(page, "Verification");
    await page.getByRole("switch", { name: "Verify with OCR" }).click();
    await openPage(page, "Logs & privacy");
    await page.getByRole("combobox", { name: "Log level" }).click();
    await page.getByRole("option", { name: "Detailed" }).click();
    await openPage(page, "Processing");
    await page.getByRole("switch", { name: "Use the GPU" }).click();

    await expect
      .poll(async () => (await settingsFile(page))?.defaults)
      .toMatchObject({
        export: { output: { folder: { Folder: EXPORTS }, template: "{name}_clean" } },
        verify: { enabled: false },
      });
    expect(await settingsFile(page)).toMatchObject({
      schema: "blurrd-settings/1",
      privacy: { log_level: "Debug" },
      processing: { hw_decode: "Software" },
    });

    await page.reload();
    await openPage(page, "Output defaults");
    await expect(page.getByTestId("default-export-folder")).toHaveText(EXPORTS);
    await expect(page.getByLabel("File name")).toHaveValue("{name}_clean");
    await openPage(page, "Detection");
    await expect(page.getByRole("switch", { name: /Voice chat/ })).not.toBeChecked();
    await expect(page.getByRole("switch", { name: /Kill feed/ })).toBeChecked();
    await openPage(page, "Verification");
    await expect(page.getByRole("switch", { name: "Verify with OCR" })).not.toBeChecked();
    await openPage(page, "Logs & privacy");
    await expect(page.getByRole("combobox", { name: "Log level" })).toHaveText("Detailed");
    await openPage(page, "Processing");
    await expect(page.getByRole("switch", { name: "Use the GPU" })).not.toBeChecked();

    // Reset goes back to next to each recording.
    await openPage(page, "Output defaults");
    await page.getByRole("button", { name: /Reset the export folder/ }).click();
    await expect(
      page.getByText("Next to each recording, in the recording's own folder."),
    ).toBeVisible();
    await expect
      .poll(async () => {
        const file = await settingsFile(page);
        return (file?.defaults as { export: { output: { folder: unknown } } } | undefined)?.export
          .output.folder;
      })
      .toBe("NextToSource");
  });

  test("show the cache and the log folder and clear the cache", async ({ page }) => {
    await openFresh(page);
    await openPage(page, "Logs & privacy");
    await expect(
      page.getByText("makes no network connections and sends no telemetry", { exact: false }),
    ).toBeVisible();
    await expect(page.getByTestId("cache-summary")).toContainText("3 recordings");
    await page.getByRole("button", { name: /Open folder/ }).click();
    await expect
      .poll(() => page.evaluate(() => window.blurrdMock?.backendSettings.logDirOpened))
      .toBe(1);
    await page.getByRole("button", { name: /^Clear/ }).click();
    await page.getByRole("dialog").getByRole("button", { name: "Clear", exact: true }).click();
    await expect(page.getByTestId("cache-summary")).toHaveText("No cached analyses.");
  });
});

test.describe("output folder", () => {
  test("the inspector shows where an export goes, and the export uses the default", async ({
    page,
  }) => {
    requireFixtureMedia();
    await openFresh(page);
    await page.evaluate((folder) => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = [folder];
    }, EXPORTS);
    await openPage(page, "Output defaults");
    await page.getByRole("button", { name: /^Change/ }).click();
    await expect(page.getByTestId("default-export-folder")).toHaveText(EXPORTS);
    await page.getByRole("button", { name: "Close settings" }).click();

    await openRecording(page, PATH);
    const panel = page.getByRole("region", { name: "Inspector" });
    await panel.getByRole("tab", { name: "Export" }).click();
    await expect(page.getByTestId("output-folder")).toHaveText(EXPORTS);
    await expect(page.getByTestId("output-preview")).toContainText("The chosen export folder.");

    // Per file, the inspector still overrides the default.
    await panel.getByRole("radio", { name: "Next to source" }).click();
    await expect(page.getByTestId("output-folder")).toHaveText("D:\\Recordings");
    await expect(page.getByTestId("output-preview")).toContainText("Next to the recording.");
    await page.evaluate((folder) => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = [folder];
    }, EXPORTS);
    await panel.getByRole("radio", { name: "Choose folder" }).click();
    await expect(page.getByTestId("output-folder")).toHaveText(EXPORTS);

    await panel.getByRole("button", { name: "Export", exact: true }).click();
    await expect(panel.getByRole("button", { name: "Export again" })).toBeEnabled();
    const output = await page.evaluate(() => {
      const call = window.blurrdMock?.calls.find((c) => c.cmd === "start_render");
      return (call?.args as { settings: { output: { folder: unknown } } } | undefined)?.settings
        .output.folder;
    });
    expect(output).toEqual({ Folder: EXPORTS });
  });
});
