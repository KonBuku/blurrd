import { expect, test, type Page } from "@playwright/test";

import { FIXTURE, INDEX_RECT, readIndex } from "../src/ipc/mock/fixture-media";
import { requireFixtureMedia } from "./fixtures";
import { canvasPixels, openFresh, openRecording } from "./preview-helpers";

const RECORDING = "D:\\Recordings\\match_ascent_01.mp4";
const LAST = FIXTURE.frames - 1;

/** A small seeded generator, so the "random" seeks are the same on every run. */
function lcg(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (Math.imul(state, 1664525) + 1013904223) >>> 0;
    return state / 2 ** 32;
  };
}

/** The frame index drawn into the frame the video element currently shows. */
function decodedIndex(page: Page): Promise<number> {
  return page
    .evaluate(
      ({ width, rect }) => {
        const video = document.querySelector("video");
        if (!video) throw new Error("no video");
        const canvas = document.createElement("canvas");
        canvas.width = video.videoWidth;
        canvas.height = video.videoHeight;
        const context = canvas.getContext("2d");
        if (!context) throw new Error("no 2d context");
        // The proxy may be smaller than the source: draw at source size.
        context.drawImage(video, 0, 0, width, (width * video.videoHeight) / video.videoWidth);
        return Array.from(context.getImageData(0, 0, rect[2], rect[3]).data);
      },
      { width: FIXTURE.width, rect: INDEX_RECT },
    )
    .then((px) => readIndex(px, INDEX_RECT[2]));
}

/** The index code read from the preview canvas (Exact patches draw it inverted). */
async function canvasIndex(page: Page): Promise<number> {
  return readIndex(await canvasPixels(page, [0, 0, INDEX_RECT[2], INDEX_RECT[3]]), INDEX_RECT[2]);
}

/** Moves the focus off the queue, so that the arrows reach the preview. */
async function focusStage(page: Page) {
  await page.evaluate(() => {
    if (document.activeElement instanceof HTMLElement) document.activeElement.blur();
  });
}

/**
 * Presses the second keys and then the frame keys (`[second back, second
 * forward, frame back, frame forward]`) to get from `from` to `to` with
 * few presses, never jumping past either end of the video.
 */
async function pressMoves(
  page: Page,
  from: number,
  to: number,
  keys: readonly [string, string, string, string],
) {
  const diff = to - from;
  let seconds = Math.round(diff / FIXTURE.fps);
  const landing = from + seconds * FIXTURE.fps;
  if (landing < 0 || landing > LAST) seconds = Math.trunc(diff / FIXTURE.fps);
  const frames = diff - seconds * FIXTURE.fps;
  const presses = [
    ...Array.from({ length: Math.abs(seconds) }, () => (seconds < 0 ? keys[0] : keys[1])),
    ...Array.from({ length: Math.abs(frames) }, () => (frames < 0 ? keys[2] : keys[3])),
  ];
  for (const press of presses) {
    // oxlint-disable-next-line no-await-in-loop
    await page.keyboard.press(press);
  }
}

/** Seeks to `frame` by one of four routes, chosen by `route`. */
async function seekBy(page: Page, route: number, from: number, frame: number) {
  const slider = page.getByRole("slider", { name: "Timeline" });
  if (route === 0) {
    // Click in the middle of the frame on the (unzoomed) timeline.
    const box = await slider.boundingBox();
    if (!box) throw new Error("timeline not laid out");
    await page.mouse.click(box.x + (box.width * (frame + 0.5)) / FIXTURE.frames, box.y + 4);
    return;
  }
  if (route === 1) {
    // Shift+arrows (one second) and arrows (one frame) on the stage.
    await focusStage(page);
    await pressMoves(page, from, frame, [
      "Shift+ArrowLeft",
      "Shift+ArrowRight",
      "ArrowLeft",
      "ArrowRight",
    ]);
    return;
  }
  if (route === 2) {
    // The timeline slider's own keys: Home, then Page Up/Down and arrows.
    await slider.focus();
    await page.keyboard.press("Home");
    await pressMoves(page, 0, frame, ["PageUp", "PageDown", "ArrowLeft", "ArrowRight"]);
    return;
  }
  // The transport: last frame, then previous-frame clicks.
  await page.getByRole("button", { name: "Go to the last frame" }).click();
  const previous = page.getByRole("button", { name: "Previous frame" });
  for (let i = 0; i < LAST - frame; i += 1) {
    // oxlint-disable-next-line no-await-in-loop
    await previous.click();
  }
}

test.describe("transport", () => {
  test.beforeEach(async ({ page }) => {
    requireFixtureMedia();
    await openFresh(page);
    await openRecording(page, RECORDING);
  });

  test("shows the decoded frame's index on 20 random seeks (checked with Exact)", async ({
    page,
  }) => {
    test.setTimeout(120_000);
    const random = lcg(24);
    const canvas = page.getByTestId("preview-canvas");
    const position = page.getByTestId("transport-position");
    await position.click();
    await page.getByRole("button", { name: "Exact" }).click();
    let from = 0;
    const seen: string[] = [];
    for (let i = 0; i < 20; i += 1) {
      const route = i % 4;
      // Transport walks stay short; the others go anywhere.
      const frame =
        route === 3 ? LAST - Math.floor(random() * 12) : Math.floor(random() * FIXTURE.frames);
      // oxlint-disable-next-line no-await-in-loop
      await seekBy(page, route, from, frame);
      // oxlint-disable-next-line no-await-in-loop
      await expect(canvas).toHaveAttribute("data-frame", String(frame));
      // oxlint-disable-next-line no-await-in-loop
      await expect(position).toHaveText(`Frame ${frame} / ${LAST}`);
      // The browser decoded that frame ...
      // oxlint-disable-next-line no-await-in-loop
      expect(await decodedIndex(page)).toBe(frame);
      // ... and the backend's Exact patch for the shown number is drawn
      // over it (the mock draws the code inverted).
      // oxlint-disable-next-line no-await-in-loop
      await expect(canvas).toHaveAttribute("data-exact-frame", String(frame));
      // oxlint-disable-next-line no-await-in-loop
      await expect.poll(() => canvasIndex(page)).toBe(~frame & 0x3ff);
      seen.push(`${["timeline", "keys", "slider", "transport"][route]}:${frame}`);
      from = frame;
    }
    test.info().annotations.push({ type: "seeks", description: seen.join(" ") });
  });

  test("keyboard shortcuts drive playback, view and zoom", async ({ page }) => {
    const canvas = page.getByTestId("preview-canvas");
    await focusStage(page);
    await page.keyboard.press("End");
    await expect(canvas).toHaveAttribute("data-frame", String(LAST));
    await page.keyboard.press("Home");
    await expect(canvas).toHaveAttribute("data-frame", "0");
    for (const press of ["ArrowRight", "ArrowRight", "ArrowRight", "Shift+ArrowRight"]) {
      // oxlint-disable-next-line no-await-in-loop
      await page.keyboard.press(press);
    }
    await expect(canvas).toHaveAttribute("data-frame", "63");
    await page.keyboard.press("Shift+ArrowLeft");
    await expect(canvas).toHaveAttribute("data-frame", "3");
    // N: the next detection start (the first kill-feed rows, frame 12).
    await page.keyboard.press("n");
    await expect(canvas).toHaveAttribute("data-frame", "12");
    await page.keyboard.press("n");
    await expect(canvas).toHaveAttribute("data-frame", "30");
    await page.keyboard.press("Shift+N");
    await expect(canvas).toHaveAttribute("data-frame", "12");

    await page.keyboard.press("b");
    await expect(page.getByRole("radio", { name: "Before" })).toBeChecked();
    await page.keyboard.press("s");
    await expect(page.getByRole("radio", { name: "Split" })).toBeChecked();
    await page.keyboard.press("a");
    await expect(page.getByRole("radio", { name: "After" })).toBeChecked();
    await page.keyboard.press("o");
    await expect(page.getByTestId("box-overlay")).toHaveCount(0);
    await page.keyboard.press("o");
    await expect(page.getByTestId("box-overlay")).toHaveCount(1);

    const zoom = page.getByRole("button", { name: /^Zoom: / });
    await page.keyboard.press("Control+Equal");
    await expect(zoom).toHaveAccessibleName("Zoom: 200 %");
    await page.keyboard.press("Control+Minus");
    await expect(zoom).toHaveAccessibleName("Zoom: 100 %");
    await page.keyboard.press("Control+0");
    await expect(zoom).toHaveAccessibleName("Zoom: Fit");

    // Forwards from the start and backwards from the end, so that the
    // shuttle neither ends nor reaches the first frame during the checks.
    await page.keyboard.press("Home");
    await page.keyboard.press("Space");
    await expect(page.getByRole("button", { name: "Pause" })).toBeVisible();
    await page.keyboard.press("k");
    await expect(page.getByRole("button", { name: "Play", exact: true })).toBeVisible();
    await page.keyboard.press("l");
    await page.keyboard.press("l");
    await expect(page.getByTestId("shuttle")).toHaveText(/Forwards\s*2×/);
    await page.keyboard.press("k");
    await expect(page.getByTestId("shuttle")).toHaveCount(0);
    await page.keyboard.press("End");
    await page.keyboard.press("j");
    await expect(page.getByTestId("shuttle")).toHaveText(/Backwards\s*1×/);
    await page.keyboard.press("j");
    await expect(page.getByTestId("shuttle")).toHaveText(/Backwards\s*2×/);
    await page.keyboard.press("k");
    await expect(page.getByTestId("shuttle")).toHaveCount(0);

    // Keys a focused control uses are left to it: arrows move the queue
    // selection, not the frame.
    await page.keyboard.press("Home");
    await expect(canvas).toHaveAttribute("data-frame", "0");
    await page.getByRole("listbox", { name: "Recordings" }).focus();
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowDown");
    await expect(page.getByTestId("transport-position")).toHaveAttribute("data-frame", "0");

    // Settings › Shortcuts lists the preview keys.
    await page.getByRole("button", { name: "Settings" }).click();
    await page.getByRole("button", { name: "Shortcuts" }).click();
    const preview = page.getByRole("region", { name: "Preview" });
    await expect(preview.getByText("Play or pause")).toBeVisible();
    await expect(preview.getByText("Space", { exact: true })).toBeVisible();
  });

  test("plays at 60 fps without long tasks", async ({ page }) => {
    const result = await page.evaluate(async () => {
      const video = document.querySelector("video");
      const canvas = document.querySelector('[data-testid="preview-canvas"]');
      const play = [...document.querySelectorAll("button")].find(
        (b) => b.getAttribute("aria-label") === "Play",
      );
      if (!video || !canvas || !play) throw new Error("preview not ready");
      const longTasks: number[] = [];
      const observer = new PerformanceObserver((list) => {
        for (const entry of list.getEntries()) longTasks.push(entry.duration);
      });
      observer.observe({ type: "longtask", buffered: false });
      const shown = new Set<string>();
      const changes = new MutationObserver(() =>
        shown.add(canvas.getAttribute("data-frame") ?? ""),
      );
      changes.observe(canvas, { attributes: true, attributeFilter: ["data-frame"] });
      const presented: { now: number; frames: number }[] = [];
      const onFrame = (now: number, meta: VideoFrameCallbackMetadata) => {
        presented.push({ now, frames: meta.presentedFrames });
        video.requestVideoFrameCallback(onFrame);
      };
      video.requestVideoFrameCallback(onFrame);
      const quality0 = video.getVideoPlaybackQuality();
      play.click();
      await new Promise<void>((resolve) => {
        video.addEventListener("ended", () => resolve(), { once: true });
      });
      observer.disconnect();
      changes.disconnect();
      const quality = video.getVideoPlaybackQuality();
      // Steady state: skip the first 10 callbacks (start-up).
      const steady = presented.slice(10);
      const first = steady[0];
      const last = steady.at(-1);
      const fps =
        first && last && last.now > first.now
          ? ((last.frames - first.frames) * 1000) / (last.now - first.now)
          : 0;
      return {
        fps,
        callbacks: presented.length,
        longTasks,
        canvasFrames: shown.size,
        dropped: quality.droppedVideoFrames - quality0.droppedVideoFrames,
      };
    });
    test.info().annotations.push({ type: "playback", description: JSON.stringify(result) });
    expect(result.fps).toBeGreaterThanOrEqual(57);
    expect(result.longTasks).toEqual([]);
    // The canvas blurs every frame the browser hands to a frame callback;
    // that is at most one per display refresh, so phase jitter between the
    // 60 fps video and the refresh skips a few (measured 216-228 of 240).
    expect(result.canvasFrames).toBeGreaterThanOrEqual(FIXTURE.frames * 0.85);
  });
});
