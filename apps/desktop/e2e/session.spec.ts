import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

/** Session storage keys of the mock backend (src/ipc/mock/index.ts). */
const SESSION_KEY = "blurrd.mock.session";
const PROBE_DURATION_KEY = "blurrd.mock.capabilitiesDelayMs";

const queue = (page: Page) => page.getByRole("listbox", { name: "Recordings" });
const options = (page: Page) => queue(page).getByRole("option");

/** Opens the app with nothing saved and an instant GPU probe. */
async function openFresh(page: Page) {
  await page.goto("/");
  await page.evaluate(
    ([key]) => {
      localStorage.clear();
      sessionStorage.clear();
      sessionStorage.setItem(key ?? "", "0");
    },
    [PROBE_DURATION_KEY],
  );
  await page.reload();
  await expect(page.getByRole("complementary", { name: "Queue" })).toBeVisible();
}

/** Every status that can be saved, one recording each. */
const SAVED = [
  { path: "D:\\Recordings\\a_queued.mp4", status: { kind: "queued" }, output: null },
  { path: "D:\\Recordings\\b_ready.mp4", status: { kind: "ready", boxes: 1284 }, output: null },
  {
    path: "D:\\Recordings\\c_done.mp4",
    status: { kind: "done", verified: true },
    output: "D:\\Recordings\\c_done_blurrd.mp4",
  },
  { path: "D:\\Recordings\\d_warnings.mp4", status: { kind: "warnings", leaks: 2 }, output: null },
  { path: "D:\\Recordings\\e_unverified.mp4", status: { kind: "unverified" }, output: null },
  {
    path: "D:\\Recordings\\f_failed.mp4",
    status: { kind: "failed", reason: "Decoder error", detail: null },
    output: null,
  },
  { path: "D:\\Recordings\\g_canceled.mp4", status: { kind: "canceled" }, output: null },
  {
    path: "D:\\Recordings\\h_missing.mp4",
    status: { kind: "done", verified: false },
    output: null,
  },
] as const;

test.describe("session restore", () => {
  test.beforeEach(async ({ page }) => {
    await openFresh(page);
  });

  test("restores the queue after a restart", async ({ page }) => {
    await page.getByRole("button", { name: "Add recordings" }).click();
    await expect(options(page)).toHaveCount(3);
    await expect
      .poll(() => page.evaluate(() => window.blurrdMock?.sessionFile()?.length ?? 0))
      .toBe(3);

    await page.reload();
    await expect(options(page)).toHaveCount(3);
    await expect(options(page).first()).toContainText("match_ascent_01.mp4");
    await expect(options(page).first()).toContainText("Queued");
    // The restored files are read again: their media facts come back.
    await expect(options(page).first()).toContainText(/\d+p[\d.]+ · \d+:\d\d · /);
  });

  test("shows the saved status of every recording, UNVERIFIED included", async ({ page }) => {
    await page.evaluate(
      ([key, entries]) => sessionStorage.setItem(String(key), JSON.stringify(entries)),
      [SESSION_KEY, SAVED] as const,
    );
    await page.reload();
    await expect(options(page)).toHaveCount(SAVED.length);

    const expected: readonly [string, string][] = [
      ["queued", "Queued"],
      ["ready", "Ready · 1,284 boxes"],
      ["done", "Done · verified clean"],
      ["warnings", "Done · 2 possible leaks"],
      ["unverified", "Unverified · review the report"],
      ["failed", "Decoder error"],
      ["canceled", "Canceled"],
      // Restored, but the file is gone.
      ["failed", "File not found"],
    ];
    for (const [index, [glyph, text]] of expected.entries()) {
      const row = options(page).nth(index);
      // Each row is independent; the loop only walks the expectations.
      // oxlint-disable-next-line no-await-in-loop
      await expect(row).toContainText(text);
      // oxlint-disable-next-line no-await-in-loop
      await expect(row.locator(`svg[data-status="${glyph}"]`)).toHaveCount(1);
    }
  });

  test("keeps no queue when restoring is turned off", async ({ page }) => {
    await page.getByRole("button", { name: "Add recordings" }).click();
    await expect(options(page)).toHaveCount(3);

    await page.keyboard.press("Control+,");
    await page.getByRole("button", { name: "General" }).click();
    const toggle = page.getByRole("switch", { name: "Restore the queue" });
    await expect(toggle).toBeChecked();
    await toggle.click();
    await expect(toggle).not.toBeChecked();
    await expect.poll(() => page.evaluate(() => window.blurrdMock?.sessionFile()?.length)).toBe(0);
    expect(await page.evaluate(() => window.blurrdMock?.settingsFile()?.general)).toMatchObject({
      restore_queue: false,
    });

    await page.reload();
    await expect(page.getByText("No recordings yet")).toBeVisible();
  });
});

test.describe("empty state and GPU card", () => {
  test("paints at once, shows probing, then the GPU results", async ({ page }) => {
    await page.goto("/");
    await page.evaluate(
      ([key]) => {
        localStorage.clear();
        sessionStorage.clear();
        sessionStorage.setItem(key ?? "", "1500");
      },
      [PROBE_DURATION_KEY],
    );
    await page.reload();

    const card = page.getByRole("region", { name: "GPU status" });
    await expect(page.getByText("Probing GPU…").first()).toBeVisible();
    await expect(card).toHaveAttribute("aria-busy", "true");
    const paint = await page.evaluate(
      () => performance.getEntriesByName("first-contentful-paint")[0]?.startTime ?? -1,
    );
    test.info().annotations.push({ type: "first-contentful-paint-ms", description: String(paint) });
    expect(paint).toBeGreaterThan(0);
    expect(paint).toBeLessThan(1500);

    await expect(page.getByText("GPU ready", { exact: true })).toBeVisible();
    await expect(page.getByText("NVENC H.264/HEVC/AV1").first()).toBeVisible();
    await expect(page.getByText("NVIDIA GeForce RTX 4080 SUPER").first()).toBeVisible();
    await expect(page.getByText("Probing GPU…")).toHaveCount(0);
    const calls = await page.evaluate(
      () => window.blurrdMock?.calls.filter((call) => call.cmd === "get_capabilities").length,
    );
    expect(calls).toBeGreaterThan(1);
  });

  test("offers browsing, adds from it and dismisses the checklist for good", async ({ page }) => {
    await openFresh(page);
    const preview = page.getByRole("region", { name: "Preview" });
    await expect(preview.getByText("MP4 · MOV · MKV — H.264 / HEVC / AV1")).toBeVisible();

    await preview.getByRole("button", { name: "Dismiss getting started" }).click();
    await expect(preview.getByText("Getting started")).toHaveCount(0);

    await preview.getByRole("button", { name: "Browse files…" }).click();
    await expect(options(page)).toHaveCount(3);
    await expect(preview.getByRole("button", { name: "Browse files…" })).toHaveCount(0);

    await page.getByRole("button", { name: "Queue menu" }).click();
    await page.getByRole("menuitem", { name: "Remove all" }).click();
    await expect(preview.getByRole("button", { name: "Browse files…" })).toBeVisible();
    await expect(preview.getByText("Getting started")).toHaveCount(0);
  });

  for (const theme of ["light", "dark"] as const) {
    test(`empty state and settings page have no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await openFresh(page);
      await expect(page.getByText("GPU ready", { exact: true })).toBeVisible();
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);

      await page.keyboard.press("Control+,");
      await page.getByRole("button", { name: "General" }).click();
      await expect(page.getByRole("heading", { name: "General" })).toBeVisible();
      expect((await new AxeBuilder({ page }).analyze()).violations).toEqual([]);
    });
  }
});
