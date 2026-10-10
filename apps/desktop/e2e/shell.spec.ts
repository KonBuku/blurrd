import { expect, test } from "@playwright/test";

test.describe("window shell", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/");
  });

  test("shows the custom title bar with window controls", async ({ page }) => {
    const titleBar = page.getByRole("banner");
    await expect(titleBar).toHaveAttribute("data-tauri-drag-region");
    await expect(titleBar.getByText("blurrd")).toBeVisible();
    await Promise.all(
      ["Minimize", "Maximize", "Close"].map((name) =>
        expect(page.getByRole("button", { name })).toBeVisible(),
      ),
    );
  });

  test("maximizes, restores and closes through the backend", async ({ page }) => {
    await page.getByRole("button", { name: "Maximize" }).click();
    await expect(page.getByRole("button", { name: "Restore" })).toBeVisible();
    expect(await page.evaluate(() => window.blurrdMock?.maximized)).toBe(true);

    await page.getByRole("button", { name: "Restore" }).click();
    await expect(page.getByRole("button", { name: "Maximize" })).toBeVisible();

    await page.getByRole("button", { name: "Close" }).click();
    await expect.poll(() => page.evaluate(() => window.blurrdMock?.closed)).toBe(true);
  });

  test("keeps the window controls reachable with the keyboard", async ({ page }) => {
    // Wait for the first render; a Tab before it lands on the empty page.
    await expect(page.getByRole("button", { name: "Minimize" })).toBeVisible();
    // Focus order starts in the title bar: queue toggle, app actions, then
    // the window controls.
    for (const name of ["Hide queue", "Command palette", "Theme", "Settings", "Minimize"]) {
      // Each Tab moves on from the focus the previous one left behind.
      // oxlint-disable-next-line no-await-in-loop
      await page.keyboard.press("Tab");
      // oxlint-disable-next-line no-await-in-loop
      await expect(page.getByRole("button", { name, exact: true })).toBeFocused();
    }
  });

  test("logs no console errors on start", async ({ page }) => {
    const errors: string[] = [];
    page.on("console", (message) => {
      if (message.type() === "error") errors.push(message.text());
    });
    await page.reload();
    await expect(page.getByRole("region", { name: "Preview" })).toBeVisible();
    expect(errors).toEqual([]);
  });
});
