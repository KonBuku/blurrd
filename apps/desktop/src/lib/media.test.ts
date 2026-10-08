import { describe, expect, it } from "vitest";

import {
  baseName,
  codecName,
  formatBytes,
  formatDuration,
  formatFps,
  formatResolution,
  looksLikeFile,
  mediaSummary,
  pathKey,
} from "./media";

describe("paths", () => {
  it("takes the last component of Windows and POSIX paths", () => {
    expect(baseName("D:\\Recordings\\match_01.mp4")).toBe("match_01.mp4");
    expect(baseName("D:/Recordings/Ranked/")).toBe("Ranked");
    expect(baseName("clip.mkv")).toBe("clip.mkv");
  });

  it("tells files from folders by the extension", () => {
    expect(looksLikeFile("D:\\Recordings\\a.mp4")).toBe(true);
    expect(looksLikeFile("D:\\Recordings\\Ranked")).toBe(false);
    expect(looksLikeFile("D:\\Rec.2026\\Ranked")).toBe(false);
  });

  it("gives two spellings of one Windows path the same key", () => {
    expect(pathKey("D:/Recordings/A.MP4")).toBe(pathKey("d:\\recordings\\a.mp4"));
    expect(pathKey("D:\\Recordings\\")).toBe(pathKey("D:\\Recordings"));
    expect(pathKey("D:\\a.mp4")).not.toBe(pathKey("D:\\b.mp4"));
  });
});

describe("media facts", () => {
  it("formats frame rates", () => {
    expect(formatFps([60, 1])).toBe("60");
    expect(formatFps([60000, 1001])).toBe("59.94");
    expect(formatFps([1, 0])).toBe("0");
  });

  it("formats durations as m:ss or h:mm:ss", () => {
    expect(formatDuration(0)).toBe("0:00");
    expect(formatDuration(151.9)).toBe("2:31");
    expect(formatDuration(3725)).toBe("1:02:05");
    expect(formatDuration(-3)).toBe("0:00");
  });

  it("formats sizes like Explorer", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(412 * 1024 * 1024)).toBe("412 MB");
    expect(formatBytes(3.2 * 1024 ** 3)).toBe("3.2 GB");
  });

  it("names codecs", () => {
    expect(codecName("hevc")).toBe("HEVC");
    expect(codecName("h264")).toBe("H.264");
    expect(codecName("theora")).toBe("THEORA");
  });

  it("builds the queue meta line", () => {
    expect(formatResolution({ height: 1080, fps: [60, 1] })).toBe("1080p60");
    expect(
      mediaSummary({
        height: 1080,
        fps: [60, 1],
        duration_s: 151,
        video_codec: "hevc",
        size: 412 * 1024 * 1024,
      }),
    ).toBe("1080p60 · 2:31 · HEVC · 412 MB");
  });
});
