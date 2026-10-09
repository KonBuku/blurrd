import { describe, expect, it } from "vitest";

import {
  MIN_VISIBLE_FRAMES,
  frameToX,
  fullView,
  panView,
  revealFrame,
  tickLabel,
  tickStep,
  xToFrame,
  zoomView,
} from "./timeline-model";

describe("timeline geometry", () => {
  it("maps frames to pixels and back, snapping to whole frames", () => {
    const view = fullView(240);
    expect(frameToX(120, view, 480)).toBe(240);
    expect(xToFrame(240, view, 480, 240)).toBe(120);
    expect(xToFrame(241.9, view, 480, 240)).toBe(120);
    expect(xToFrame(-5, view, 480, 240)).toBe(0);
    expect(xToFrame(9999, view, 480, 240)).toBe(239);
    // Every frame start maps back to its frame at any width.
    for (const width of [97, 480, 1333]) {
      for (let f = 0; f < 240; f += 1) {
        expect(xToFrame(frameToX(f, view, width) + 1e-6, view, width, 240)).toBe(f);
      }
    }
  });

  it("zooms around an anchor and stays inside the video", () => {
    const zoomed = zoomView(fullView(1000), 250, 4, 1000);
    expect(zoomed.end - zoomed.start).toBeCloseTo(250);
    // The anchor keeps its relative position.
    expect((250 - zoomed.start) / (zoomed.end - zoomed.start)).toBeCloseTo(0.25);
    const max = zoomView(zoomed, 250, 1000, 1000);
    expect(max.end - max.start).toBe(MIN_VISIBLE_FRAMES);
    expect(zoomView(zoomed, 250, 0.001, 1000)).toEqual(fullView(1000));
    expect(fullView(0)).toEqual({ start: 0, end: 1 });
  });

  it("pans and reveals within the video", () => {
    const view = { start: 100, end: 200 };
    expect(panView(view, -500, 1000)).toEqual({ start: 0, end: 100 });
    expect(panView(view, 5000, 1000)).toEqual({ start: 900, end: 1000 });
    expect(revealFrame(view, 150, 1000)).toBe(view);
    expect(revealFrame(view, 250, 1000)).toEqual({ start: 151, end: 251 });
    expect(revealFrame(view, 20, 1000)).toEqual({ start: 20, end: 120 });
  });

  it("chooses ruler steps at least 72 px apart", () => {
    expect(tickStep(fullView(60 * 60), 720, 60)).toBe(10);
    expect(tickStep({ start: 0, end: 60 }, 1000, 60)).toBe(0.1);
    expect(tickLabel(75, 5)).toBe("1:15");
    expect(tickLabel(1.5, 0.5)).toBe("0:01.5");
  });
});
