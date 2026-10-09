import { describe, expect, it } from "vitest";

import { centerForAnchor, clampCenter, fitScale, stageBox, stepZoom, toSource } from "./stage";

/** Source formats of the robustness matrix: 720p..4K, 4:3, 16:10, ultrawide. */
const SOURCES = [
  { width: 1280, height: 720 },
  { width: 1920, height: 1080 },
  { width: 2560, height: 1440 },
  { width: 3840, height: 2160 },
  { width: 1440, height: 1080 },
  { width: 1680, height: 1050 },
  { width: 3440, height: 1440 },
  { width: 640, height: 360 },
];
const STAGES = [
  { width: 480, height: 300 },
  { width: 1100, height: 619 },
  { width: 1920, height: 1300 },
];
const ZOOMS = ["fit", 0.25, 0.5, 1, 2, 4] as const;

describe("stageBox", () => {
  it("letterboxes the frame when fitting", () => {
    const box = stageBox({ width: 1000, height: 1000 }, { width: 1920, height: 1080 }, "fit", null);
    expect(box.scale).toBeCloseTo(1000 / 1920, 9);
    expect(box.left).toBe(0);
    expect(box.top).toBe(Math.round(500 - 540 * (1000 / 1920)));
  });

  it("maps source pixels to the stage and back within 1 source pixel at every zoom", () => {
    for (const source of SOURCES) {
      for (const stage of STAGES) {
        for (const zoom of ZOOMS) {
          const box = stageBox(stage, source, zoom, {
            x: source.width * 0.7,
            y: source.height * 0.3,
          });
          for (const p of [
            { x: 0, y: 0 },
            { x: source.width - 1, y: source.height - 1 },
            { x: source.width * 0.81, y: source.height * 0.07 },
          ]) {
            const screen = { x: box.left + p.x * box.scale, y: box.top + p.y * box.scale };
            const back = toSource(box, screen);
            expect(Math.abs(back.x - p.x)).toBeLessThan(1e-6);
            expect(Math.abs(back.y - p.y)).toBeLessThan(1e-6);
          }
          // A frame wider than the stage always covers it.
          const covers = box.left <= 0 && box.left + box.width >= stage.width - 1;
          expect(box.width < stage.width || covers).toBe(true);
        }
      }
    }
  });

  it("keeps the zoom anchor under the pointer", () => {
    const stage = { width: 1100, height: 619 };
    const source = { width: 1920, height: 1080 };
    const before = stageBox(stage, source, 1, { x: 900, y: 500 });
    const pointer = { x: 300, y: 200 };
    const anchor = toSource(before, pointer);
    const center = clampCenter(centerForAnchor(anchor, pointer, stage, 2), stage, source, 2);
    const after = stageBox(stage, source, 2, center);
    const moved = toSource(after, pointer);
    expect(Math.abs(moved.x - anchor.x)).toBeLessThanOrEqual(0.5);
    expect(Math.abs(moved.y - anchor.y)).toBeLessThanOrEqual(0.5);
  });

  it("survives an unmeasured stage", () => {
    const box = stageBox({ width: 0, height: 0 }, { width: 1920, height: 1080 }, "fit", null);
    expect(Number.isFinite(box.left) && Number.isFinite(box.top)).toBe(true);
  });
});

describe("zoom helpers", () => {
  it("fits and steps through the levels", () => {
    expect(fitScale({ width: 960, height: 540 }, { width: 1920, height: 1080 })).toBe(0.5);
    expect(stepZoom(1, 1)).toBe(2);
    expect(stepZoom(1, -1)).toBe(0.5);
    expect(stepZoom(0.6, 1)).toBe(1);
    expect(stepZoom(4, 1)).toBe(4);
    expect(stepZoom(0.25, -1)).toBe(0.25);
  });
});
