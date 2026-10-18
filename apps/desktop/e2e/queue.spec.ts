import { AxeBuilder } from "@axe-core/playwright";
import { expect, test, type Page } from "@playwright/test";

import { chooseTheme } from "./fixtures";

/** Number of files the scrolling test drops (acceptance M23: 200 files). */
const MANY = 200;

/** Opens the app with an empty queue, a clean layout and an instant probe. */
async function openApp(page: Page) {
  await page.goto("/");
  await page.evaluate(() => {
    localStorage.clear();
    sessionStorage.clear();
  });
  await page.reload();
  await expect(page.getByRole("complementary", { name: "Queue" })).toBeVisible();
  await page.evaluate(() => {
    if (window.blurrdMock) window.blurrdMock.probeDelayMs = 0;
  });
}

/** Registers a folder with `count` videos in the mock file system and drops it. */
async function dropFolder(page: Page, count: number) {
  await page.evaluate(async (total) => {
    const backend = window.blurrdMock;
    if (!backend) throw new Error("mock backend missing");
    const files = Array.from(
      { length: total },
      (_, index) => `E:\\Matches\\match_${String(index + 1).padStart(3, "0")}.mp4`,
    );
    backend.folders.set("E:\\Matches", files);
    await backend.drop(["E:\\Matches"]);
  }, count);
}

const options = (page: Page) =>
  page.getByRole("listbox", { name: "Recordings" }).getByRole("option");

/** Frame statistics of one {@link measureFrames} run. */
interface FrameStats {
  readonly frames: number;
  readonly fps: number;
  readonly p95: number;
  readonly longTasks: readonly number[];
  readonly domRows: number;
}

/**
 * Runs in the page: records frame intervals and long tasks for 300 frames,
 * or, with `scroll`, while scrolling the queue list 30 px per frame from
 * the top to the bottom.
 */
async function measureFrames(scroll: boolean): Promise<FrameStats> {
  const list = document.querySelector<HTMLElement>('[role="listbox"][aria-label="Recordings"]');
  if (!list) throw new Error("queue list missing");
  list.scrollTop = 0;
  const longTasks: number[] = [];
  const observer = new PerformanceObserver((entries) => {
    for (const entry of entries.getEntries()) longTasks.push(entry.duration);
  });
  observer.observe({ type: "longtask", buffered: false });
  const intervals: number[] = [];
  await new Promise<void>((resolve) => {
    let last = 0;
    const step = (now: number) => {
      if (last !== 0) intervals.push(now - last);
      last = now;
      const done = scroll
        ? list.scrollTop + list.clientHeight >= list.scrollHeight - 1
        : intervals.length >= 300;
      if (scroll) list.scrollTop += 30;
      if (done) resolve();
      else requestAnimationFrame(step);
    };
    requestAnimationFrame(step);
  });
  observer.disconnect();
  const sorted = intervals.toSorted((a, b) => a - b);
  const total = intervals.reduce((sum, value) => sum + value, 0);
  return {
    frames: intervals.length,
    fps: (intervals.length * 1000) / total,
    p95: sorted[Math.floor(sorted.length * 0.95)] ?? 0,
    longTasks,
    domRows: list.querySelectorAll('[role="option"]').length,
  };
}

test.describe("import", () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test("adds recordings from the dialog and lists unsupported files in a toast", async ({
    page,
  }) => {
    await page.getByRole("button", { name: "Add recordings" }).click();
    await expect(options(page)).toHaveCount(3);
    await expect(page.getByText("1 file can't be added")).toBeVisible();
    await expect(page.getByText("thumbnail.png — not a supported video")).toBeVisible();
    await expect(page.getByLabel("3 recordings")).toBeVisible();
    await expect(options(page).first()).toContainText("match_ascent_01.mp4");
    // Its analysis is in the (mocked) cache, so it is ready at once.
    await expect(options(page).first()).toContainText("Ready");
  });

  test("lists a file without a cached analysis as queued", async ({ page }) => {
    await page.evaluate(() => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = ["D:\\Recordings\\new_fresh.mp4"];
    });
    await page.getByRole("button", { name: "Add recordings" }).click();
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toContainText("Queued");
  });

  test("adds a folder from the queue menu and with Ctrl+Shift+O", async ({ page }) => {
    await page.evaluate(() => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = ["D:\\Recordings\\Ranked"];
    });
    await page.getByRole("button", { name: "Queue menu" }).click();
    await page.getByRole("menuitem", { name: /Add folder/ }).click();
    await expect(options(page)).toHaveCount(3);

    await page.evaluate(() => {
      if (window.blurrdMock) window.blurrdMock.dialogResult = ["D:\\Recordings\\extra.mkv"];
    });
    await page.keyboard.press("Control+Shift+O");
    await expect(options(page)).toHaveCount(4);
  });

  test("shows the drop overlay while dragging and adds what is dropped", async ({ page }) => {
    const paths = ["D:\\a.mp4", "D:\\b.mp4", "D:\\c.mov", "D:\\d.mkv"];
    await page.evaluate((dragged) => window.blurrdMock?.dragEnter(dragged), paths);
    await expect(page.getByTestId("drop-overlay")).toContainText("Drop to add 4 items");
    await page.evaluate(() => window.blurrdMock?.dragLeave());
    await expect(page.getByTestId("drop-overlay")).toHaveCount(0);

    await page.evaluate((dropped) => window.blurrdMock?.drop(dropped), paths);
    await expect(page.getByTestId("drop-overlay")).toHaveCount(0);
    await expect(options(page)).toHaveCount(4);
  });

  test("adds files passed to a second start", async ({ page }) => {
    await page.evaluate(() => window.blurrdMock?.secondInstance(["D:\\Recordings\\opened.mp4"]));
    await expect(options(page)).toHaveCount(1);
    await expect(options(page).first()).toContainText("opened.mp4");
  });

  test("reports a failed probe and leaves no placeholder", async ({ page }) => {
    await page.evaluate(() => {
      if (window.blurrdMock) {
        window.blurrdMock.probeError = {
          kind: "Io",
          message: "The disk is not ready.",
          detail: null,
        };
      }
    });
    await page.getByRole("button", { name: "Add recordings" }).click();
    await expect(page.getByText("The files could not be read")).toBeVisible();
    await expect(page.getByText("No recordings yet")).toBeVisible();
  });
});

test.describe("queue", () => {
  test.beforeEach(async ({ page }) => {
    await openApp(page);
  });

  test(`keeps ${MANY} files virtualized and scrolls at 60 fps @perf`, async ({ page }) => {
    await dropFolder(page, MANY);
    await expect(page.getByLabel(`${MANY} recordings`)).toBeVisible();
    const rendered = await options(page).count();
    expect(rendered).toBeLessThan(30);

    // Frame cadence of the idle page first (the machine's baseline: CI
    // runners and busy desktops do not deliver a perfect 60 Hz), then the
    // list scrolled from top to bottom at 1,800 px/s, one step per frame,
    // with the main-thread time the scrolling costs (Chrome's TaskDuration).
    const cdp = await page.context().newCDPSession(page);
    await cdp.send("Performance.enable");
    const taskSeconds = async () => {
      const { metrics } = await cdp.send("Performance.getMetrics");
      return metrics.find((metric) => metric.name === "TaskDuration")?.value ?? 0;
    };
    const idle = await page.evaluate(measureFrames, false);
    const before = await taskSeconds();
    const scroll = await page.evaluate(measureFrames, true);
    const busyMs = ((await taskSeconds()) - before) * 1000;
    const stats = { idle, scroll, busyMsPerFrame: busyMs / scroll.frames };
    test.info().annotations.push({ type: "scroll", description: JSON.stringify(stats) });

    expect(scroll.frames).toBeGreaterThan(200);
    expect(scroll.longTasks).toEqual([]);
    expect(scroll.domRows).toBeLessThan(30);
    // As smooth as the idle page, and far inside the 16.7 ms frame budget.
    expect(scroll.fps).toBeGreaterThanOrEqual(Math.min(idle.fps, 60) * 0.97);
    expect(stats.busyMsPerFrame).toBeLessThan(8);
    await expect(options(page).last()).toContainText(`match_${MANY}.mp4`);
  });

  test("selects, extends and removes with the keyboard", async ({ page }) => {
    await dropFolder(page, 5);
    const list = page.getByRole("listbox", { name: "Recordings" });
    await list.focus();
    await page.keyboard.press("ArrowDown");
    await expect(options(page).nth(0)).toHaveAttribute("aria-selected", "true");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    await expect(list.locator('[aria-selected="true"]')).toHaveCount(3);
    await page.keyboard.press("Delete");
    await expect(options(page)).toHaveCount(2);
    await expect(options(page).first()).toContainText("match_004.mp4");

    await page.keyboard.press("Control+A");
    await expect(list.locator('[aria-selected="true"]')).toHaveCount(2);
    await page.keyboard.press("Delete");
    await expect(page.getByText("No recordings yet")).toBeVisible();
  });

  test("selects with Ctrl and Shift clicks and removes from the context menu", async ({ page }) => {
    await dropFolder(page, 6);
    await options(page).nth(1).click();
    await options(page)
      .nth(3)
      .click({ modifiers: ["Shift"] });
    await options(page)
      .nth(5)
      .click({ modifiers: ["Control"] });
    const list = page.getByRole("listbox", { name: "Recordings" });
    await expect(list.locator('[aria-selected="true"]')).toHaveCount(4);

    await options(page).nth(2).click({ button: "right" });
    await page.getByRole("menuitem", { name: "Remove 4 recordings" }).click();
    await expect(options(page)).toHaveCount(2);

    // Right-clicking an unselected row acts on that row alone.
    await options(page).nth(0).click({ button: "right" });
    await expect(page.getByRole("menuitem", { name: "Copy path" })).toBeVisible();
    await page.keyboard.press("Escape");
  });

  test("filters long queues and sorts from the menu", async ({ page }) => {
    await dropFolder(page, 8);
    await expect(page.getByRole("searchbox", { name: "Filter recordings" })).toHaveCount(0);
    await page.evaluate(() => window.blurrdMock?.drop(["D:\\Recordings\\aaa_last.mp4"]));
    const filter = page.getByRole("searchbox", { name: "Filter recordings" });
    await expect(filter).toBeVisible();
    await filter.fill("aaa");
    await expect(options(page)).toHaveCount(1);
    await filter.fill("zzz");
    await expect(page.getByText("No recordings match “zzz”.")).toBeVisible();
    await filter.fill("");

    await page.getByRole("button", { name: "Queue menu" }).click();
    await page.getByRole("menuitem", { name: "Name" }).click();
    await expect(options(page).first()).toContainText("aaa_last.mp4");
    await page.getByRole("button", { name: "Queue menu" }).click();
    await page.getByRole("menuitem", { name: "Remove all" }).click();
    await expect(page.getByText("No recordings yet")).toBeVisible();
  });

  for (const theme of ["light", "dark"] as const) {
    test(`queue with recordings has no axe violations (${theme})`, async ({ page }) => {
      await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
      await chooseTheme(page, theme);
      await dropFolder(page, 12);
      await options(page).nth(1).click();
      await page.getByRole("listbox", { name: "Recordings" }).focus();
      const results = await new AxeBuilder({ page }).analyze();
      expect(results.violations).toEqual([]);
    });
  }
});
