import { describe, expect, it } from "vitest";

import { mockProject } from "@/ipc/mock/project";

import { fallbackSource, initialSource, problemText } from "./source";

describe("preview source", () => {
  const hevc = mockProject("D:\\Recordings\\clip_hevc.mp4");
  const plain = mockProject("D:\\Recordings\\clip.mp4");

  it("plays the source when the browser can decode it", () => {
    expect(initialSource(hevc, () => "probably")).toEqual({ url: hevc.media_url, proxy: false });
    expect(initialSource(plain, () => "")).toEqual({ url: plain.media_url, proxy: false });
  });

  it("starts with the proxy when the codec is not supported", () => {
    expect(initialSource(hevc, () => "")).toEqual({ url: hevc.proxy_url, proxy: true });
  });

  it("falls back to the proxy once after an error", () => {
    const first = { url: hevc.media_url, proxy: false };
    const proxy = fallbackSource(hevc, first);
    expect(proxy).toEqual({ url: hevc.proxy_url, proxy: true });
    expect(proxy && fallbackSource(hevc, proxy)).toBeNull();
    expect(fallbackSource(plain, { url: plain.media_url, proxy: false })).toBeNull();
  });

  it("explains how to make a file previewable", () => {
    expect(problemText(hevc)).toContain("HEVC Video Extensions");
    expect(problemText(plain)).toContain("Analyzing it creates a preview copy");
  });
});
