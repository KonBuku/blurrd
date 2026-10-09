import { describe, expect, it } from "vitest";

import { FrameClock, formatTimecode } from "./frames";

describe("FrameClock", () => {
  // 60 fps in a millisecond time base (WebM): 0, 17, 33, 50, ... ms.
  const webm = new FrameClock(
    Array.from({ length: 240 }, (_, i) => Math.round((i * 1000) / 60)),
    [1, 1000],
    [60, 1],
  );

  it("maps every frame's seek time back to the frame", () => {
    for (let f = 0; f < webm.count; f += 1) expect(webm.frameAt(webm.seekTime(f))).toBe(f);
  });

  it("maps media times to the frame on screen", () => {
    expect(webm.frameAt(0)).toBe(0);
    expect(webm.frameAt(0.0165)).toBe(0);
    // The browser may report the frame start with float noise.
    expect(webm.frameAt(0.017 - 1e-9)).toBe(1);
    expect(webm.frameAt(1.0)).toBe(60);
    expect(webm.frameAt(99)).toBe(239);
    expect(webm.frameAt(-1)).toBe(0);
  });

  it("handles 59.94 fps, an offset first timestamp and variable frame rates", () => {
    const ntsc = new FrameClock(
      Array.from({ length: 600 }, (_, i) => 1001 + i * 1001),
      [1, 60000],
      [60000, 1001],
    );
    for (const f of [0, 1, 299, 599]) expect(ntsc.frameAt(ntsc.seekTime(f))).toBe(f);
    expect(ntsc.startOf(1)).toBeCloseTo(1001 / 60000, 9);
    const vfr = new FrameClock([0, 10, 50, 51, 120], [1, 1000], [30, 1]);
    for (let f = 0; f < 5; f += 1) expect(vfr.frameAt(vfr.seekTime(f))).toBe(f);
    expect(vfr.seekTime(4)).toBeCloseTo(0.12 + 1 / 60, 9);
  });

  it("clamps frames and reports the duration", () => {
    expect(webm.clamp(-5)).toBe(0);
    expect(webm.clamp(1e9)).toBe(239);
    expect(webm.duration).toBeCloseTo(3.983 + 1 / 60, 3);
    expect(new FrameClock([], [1, 1000], [60, 1]).frameAt(1)).toBe(0);
  });
});

describe("formatTimecode", () => {
  it("formats hours, minutes, seconds and hundredths", () => {
    expect(formatTimecode(0)).toBe("00:00:00.00");
    expect(formatTimecode(72.34)).toBe("00:01:12.34");
    expect(formatTimecode(151.08)).toBe("00:02:31.08");
    expect(formatTimecode(3600 + 59.999)).toBe("01:00:59.99");
    expect(formatTimecode(-1)).toBe("00:00:00.00");
  });
});
