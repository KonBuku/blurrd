import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { chooseTheme, requireFixtureMedia } from "./fixtures";
import { openFresh, openRecording } from "./preview-helpers";

const PATH = "D:\\Recordings\\match_ascent_01.mp4";
const MORE = ["D:\\Recordings\\match_bind_02.mp4", "D:\\Recordings\\match_lotus_03.mp4"];

/** The inspector frame. */
function inspector(page: Page) {
  return page.getByRole("region", { name: "Inspector" });
}

/** The processing card. */
function card(page: Page) {
  return page.getByRole("region", { name: "Processing" });
}

/** Sets how long each mock job runs. */
function jobDuration(page: Page, ms: number) {
  return page.evaluate((duration) => {
    if (window.blurrdMock) window.blurrdMock.jobs.durationMs = duration;
  }, ms);
}

/** The taskbar states the window was given. */
function taskbar(page: Page) {
  return page.evaluate(() => window.blurrdMock?.taskbar ?? []);
}

/** A share as a percentage with one decimal. */
function percent(share: number): string {
  return `${(share * 100).toFixed(1)}%`;
}

/** Starts the export of the shown file from the inspector footer. */
async function exportShown(page: Page) {
  await inspector(page).getByRole("tab", { name: "Export" }).click();
  await inspector(page).getByRole("button", { name: "Export", exact: true }).click();
}

test.describe("processing", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, PATH);
  });

  test("card: stages, figures, taskbar, and cancel during a stage", async ({ page }) => {
    await jobDuration(page, 4000);
    await exportShown(page);
    await expect(card(page).getByRole("heading")).toHaveText("Exporting match_ascent_01.mp4");
    // Edits changed frames since the last verification: they are verified first.
    await expect(card(page).getByRole("list", { name: "Stages" }).getByRole("listitem")).toHaveText(
      [/Decode/, /OCR/, /Verify/, /Blur \+ encode/, /Mux/],
    );
    await expect(card(page).locator('[aria-current="step"]')).toHaveText("Blur + encode", {
      timeout: 10_000,
    });
    await expect(page.getByTestId("processing-figures")).toHaveText(
      /^390 fps · [\d,]+ \/ [\d,]+ frames · \d+:\d\d left$/,
    );
    const states = await taskbar(page);
    expect(states.some((state) => state.status === "normal" && (state.progress ?? 0) > 50)).toBe(
      true,
    );

    await card(page).getByRole("button", { name: "Cancel" }).click();
    await expect(page.getByRole("listbox", { name: "Recordings" })).toContainText("Canceled");
    await expect(card(page)).toBeHidden();
    expect((await taskbar(page)).at(-1)).toEqual({ status: "none", progress: 0 });
    await expect(
      inspector(page).getByRole("button", { name: "Export", exact: true }),
    ).toBeEnabled();
  });

  test("pause holds the job, resume finishes it", async ({ page }) => {
    await jobDuration(page, 2500);
    await exportShown(page);
    await expect(page.getByTestId("processing-figures")).toContainText("frames");
    await card(page).getByRole("button", { name: "Pause" }).click();
    await expect(card(page).getByRole("heading")).toHaveText(
      "Paused · exporting match_ascent_01.mp4",
    );
    expect((await taskbar(page)).at(-1)?.status).toBe("paused");
    const held = await page.getByTestId("processing-figures").textContent();
    await page.waitForTimeout(400);
    await expect(page.getByTestId("processing-figures")).toHaveText(held ?? "");
    await card(page).getByRole("button", { name: "Resume" }).click();
    await expect(page.getByRole("listbox", { name: "Recordings" })).toContainText(
      "Unverified · review the report",
      { timeout: 10_000 },
    );
  });

  test("export N files with Ctrl+E: run queue, queue progress, one notification", async ({
    page,
  }) => {
    await page.evaluate((paths) => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = paths;
    }, MORE);
    await page.getByRole("button", { name: "Add recordings" }).click();
    const list = page.getByRole("listbox", { name: "Recordings" });
    await expect(list.getByRole("option")).toHaveCount(3);
    await list.focus();
    await page.keyboard.press("Control+A");
    await expect(inspector(page).getByRole("button", { name: "Export 3 files" })).toBeEnabled();

    await jobDuration(page, 700);
    await page.evaluate(() => {
      if (window.blurrdMock) window.blurrdMock.focused = false;
    });
    await page.keyboard.press("Control+E");
    const progress = page.getByTestId("batch-progress");
    await expect(progress).toContainText("of 3 files done");
    await expect(list).toContainText("Waiting to export");
    await expect(progress).toBeHidden({ timeout: 15_000 });

    const calls = await page.evaluate(() =>
      (window.blurrdMock?.calls ?? []).filter((call) => call.cmd.startsWith("start_")),
    );
    expect(calls.map((call) => call.cmd)).toEqual(["start_run"]);
    const notifications = await page.evaluate(() => window.blurrdMock?.notifications ?? []);
    expect(notifications).toHaveLength(1);
    expect(notifications[0]?.title).toBe("Export finished");
    expect(notifications[0]?.body).toMatch(/^3 exported/);

    // The empty state lists this session's exports.
    await page.getByRole("button", { name: "Queue menu" }).click();
    await page.getByRole("menuitem", { name: "Remove all" }).click();
    const recent = page.getByRole("region", { name: "Recent exports" });
    await expect(recent.getByRole("listitem")).toHaveCount(3);
    await recent.getByRole("button", { name: /^Show match_lotus_03_blurrd/ }).click();
    const revealed = await page.evaluate(() => window.blurrdMock?.revealed ?? []);
    expect(revealed).toEqual([expect.stringMatching(/match_lotus_03_blurrd.*\.mp4$/)]);
  });

  test("a failure shows its message, then the details to copy", async ({ page, context }) => {
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await page.evaluate(() => {
      if (window.blurrdMock) {
        window.blurrdMock.jobs.failNext = {
          kind: "Encode",
          message: "The encoder stopped.",
          detail: "nvenc: out of memory (code 10)",
        };
      }
    });
    await exportShown(page);
    const toast = page.locator("[data-sonner-toast]").filter({ hasText: "could not be exported" });
    await expect(toast).toContainText("The encoder stopped.");
    await toast.getByRole("button", { name: "Details" }).click();

    const dialog = page.getByRole("dialog", { name: "match_ascent_01.mp4 could not be exported" });
    await expect(dialog).toContainText("The encoder stopped.");
    await expect(dialog.getByRole("textbox", { name: "Technical details" })).toHaveValue(
      "nvenc: out of memory (code 10)",
    );
    const axe = await new AxeBuilder({ page }).include('[role="dialog"]').analyze();
    expect(axe.violations).toEqual([]);
    await dialog.getByRole("button", { name: "Copy details" }).click();
    await expect(dialog.getByRole("button", { name: "Copied" })).toBeVisible();
    expect(await page.evaluate(() => navigator.clipboard.readText())).toContain(
      "nvenc: out of memory (code 10)",
    );
    await dialog.getByRole("button", { name: "Close" }).click();

    // The queue keeps the failure; its menu opens the same details.
    const list = page.getByRole("listbox", { name: "Recordings" });
    await expect(list).toContainText("The encoder stopped.");
    await list.getByRole("option").first().click({ button: "right" });
    await page.getByRole("menuitem", { name: "Show error details…" }).click();
    await expect(page.getByRole("dialog")).toContainText("nvenc: out of memory (code 10)");
  });

  for (const scheme of ["light", "dark"] as const) {
    test(`processing card and queue progress have no axe violations (${scheme})`, async ({
      page,
    }) => {
      await page.emulateMedia({ colorScheme: scheme });
      await chooseTheme(page, scheme);
      await jobDuration(page, 5000);
      await exportShown(page);
      await expect(card(page).locator('[aria-current="step"]')).toBeVisible();
      await expect(page.getByTestId("batch-progress")).toBeVisible();
      const axe = await new AxeBuilder({ page })
        .include('[data-testid="processing-card"]')
        .include('[data-testid="batch-progress"]')
        .analyze();
      expect(axe.violations).toEqual([]);
    });
  }

  test("keeps processing while the window is hidden (measured)", async ({ page }) => {
    // Wall time of an export with the window shown, then with the page
    // treated like a hidden window: visibility "hidden", no animation
    // frames, timers at most once a second. The frontend reacts to job
    // events, never to timers or frames, so both take as long as the job.
    const run = async () => {
      const t0 = Date.now();
      // A click from inside the page, which needs no animation frame.
      await page.evaluate(() =>
        document.querySelector<HTMLButtonElement>('button[aria-keyshortcuts="Control+E"]')?.click(),
      );
      await expect
        .poll(
          () => page.evaluate(() => document.querySelector('[data-testid="processing-card"]')),
          {
            intervals: [20],
          },
        )
        .not.toBeNull();
      await expect
        .poll(
          () => page.evaluate(() => document.querySelector('[data-testid="processing-card"]')),
          { timeout: 15_000, intervals: [20] },
        )
        .toBeNull();
      return Date.now() - t0;
    };
    await inspector(page).getByRole("tab", { name: "Export" }).click();
    // The first export verifies the edited ranges too; measure the later ones.
    await jobDuration(page, 100);
    await run();
    await jobDuration(page, 1500);
    const shown = await run();

    await page.evaluate(() => {
      Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
      Object.defineProperty(document, "hidden", { value: true, configurable: true });
      document.dispatchEvent(new Event("visibilitychange"));
      window.requestAnimationFrame = () => 0;
      const timeout = window.setTimeout.bind(window);
      window.setTimeout = ((handler: TimerHandler, ms?: number, ...rest: unknown[]) =>
        timeout(handler, Math.max(ms ?? 0, 1000), ...rest)) as typeof window.setTimeout;
      const interval = window.setInterval.bind(window);
      window.setInterval = ((handler: TimerHandler, ms?: number, ...rest: unknown[]) =>
        interval(handler, Math.max(ms ?? 0, 1000), ...rest)) as typeof window.setInterval;
    });
    const before = (await taskbar(page)).length;
    const hidden = await run();
    const updates = (await taskbar(page)).length - before;
    test.info().annotations.push({
      type: "hidden-window",
      description: `shown=${shown}ms hidden=${hidden}ms taskbarUpdates=${updates}`,
    });
    expect(hidden).toBeLessThan(shown * 1.1 + 100);
    expect(updates).toBeGreaterThan(10);
  });

  test("handles progress events with little main-thread time (measured) @perf", async ({
    page,
  }) => {
    // The engine runs outside the webview; what the UI adds is the time
    // its main thread spends on job events (20 a second, plan M22): script
    // for the stores and React, style and layout for the changed text.
    // Measured over a 3 s export with the card on screen.
    await inspector(page).getByRole("tab", { name: "Export" }).click();
    await jobDuration(page, 100);
    await inspector(page).getByRole("button", { name: "Export", exact: true }).click();
    await expect(inspector(page).getByRole("button", { name: "Export again" })).toBeEnabled();

    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    const NAMES = ["ScriptDuration", "RecalcStyleDuration", "LayoutDuration", "TaskDuration"];
    const sample = async () => {
      const { metrics } = await cdp.send("Performance.getMetrics");
      return NAMES.map((name) => metrics.find((metric) => metric.name === name)?.value ?? 0);
    };
    await page.evaluate(() => {
      const long: number[] = [];
      new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) long.push(entry.duration);
      }).observe({ type: "longtask" });
      (window as unknown as { longTasks: number[] }).longTasks = long;
    });
    await jobDuration(page, 3000);
    const before = await sample();
    const t0 = Date.now();
    await inspector(page).getByRole("button", { name: "Export again" }).click();
    await expect(card(page)).toBeVisible();
    await expect(card(page)).toBeHidden({ timeout: 10_000 });
    const wall = (Date.now() - t0) / 1000;
    const after = await sample();
    const [script = 0, style = 0, layout = 0, task = 0] = after.map(
      (value, index) => (value - (before[index] ?? 0)) / wall,
    );
    const longTasks = await page.evaluate(
      () => (window as unknown as { longTasks: number[] }).longTasks,
    );
    test.info().annotations.push({
      type: "main-thread",
      description: `wall=${wall.toFixed(2)}s script=${percent(script)} style=${percent(style)} layout=${percent(layout)} task=${percent(task)} longTasks=${longTasks.length}`,
    });
    // Dev build (React development mode); a release build costs less.
    expect(script + style + layout).toBeLessThan(0.15);
    expect(longTasks.filter((duration) => duration > 100)).toEqual([]);
  });
});
