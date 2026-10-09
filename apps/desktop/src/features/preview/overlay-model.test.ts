import { describe, expect, it } from "vitest";

import type { Region } from "@/bindings/Region";
import { mockTracks, mockVerifyReport } from "@/ipc/mock/project";
import { IntervalIndex } from "@/lib/interval-index";

import { nextMark, overlayItems } from "./overlay-model";

const PATH = String.raw`D:\Recordings\match_ascent_01.mp4`;
const ALL = new Set<Region>(["KillFeed", "Banner", "VoiceChat", "TextChat"]);

describe("overlayItems", () => {
  const tracks = mockTracks(PATH);
  const index = new IntervalIndex(240, tracks.tracks, tracks.overrides);
  const report = mockVerifyReport(PATH);

  it("lists boxes, then inactive edits, then leaks of the frame", () => {
    const items = overlayItems(31, index, ALL, tracks, report);
    const kinds = items.map((item) => item.kind);
    expect(kinds.indexOf("unbound")).toBeGreaterThan(kinds.lastIndexOf("box"));
    expect(kinds.indexOf("leak")).toBeGreaterThan(kinds.lastIndexOf("unbound"));
    expect(items.filter((item) => item.kind === "unbound")).toHaveLength(1);
    const leak = items.find((item) => item.kind === "leak");
    expect(leak).toMatchObject({ key: "leak:0", open: true, region: "KillFeed" });
    expect(leak?.label).toBe("Kill feed · readable name (open) · frames 30–34");
  });

  it("describes a box by region, name kind and span", () => {
    const manual = overlayItems(5, index, ALL, tracks, null).find(
      (item) => item.region === "Manual",
    );
    expect(manual?.kind).toBe("box");
    expect(manual?.label).toMatch(/^Manual box · manual box · frames 0–239$/);
    expect(manual?.key).toMatch(/^edit:/);
  });

  it("keeps keys stable across frames", () => {
    const [a, b] = [100, 101].map((frame) =>
      overlayItems(frame, index, ALL, tracks, null).map((item) => item.key),
    );
    expect(a?.filter((key) => b?.includes(key)).length).toBeGreaterThan(0);
    expect(new Set(a).size).toBe(a?.length);
  });

  it("leaves fixed leaks out and marks information as not open", () => {
    expect(overlayItems(120, index, ALL, tracks, report).some((item) => item.kind === "leak")).toBe(
      false,
    );
    const info = overlayItems(42, index, ALL, tracks, report).find((item) => item.kind === "leak");
    expect(info).toMatchObject({ open: false, region: null });
    expect(info?.label).toBe(
      "Outside the HUD · name outside the blurred regions (information) · frames 40–44",
    );
  });

  it("skips disabled regions", () => {
    const items = overlayItems(100, index, new Set<Region>(), tracks, null);
    expect(items.every((item) => item.region === "Manual" || item.kind !== "box")).toBe(true);
  });
});

describe("nextMark", () => {
  const tracks = mockTracks(PATH);
  const index = new IntervalIndex(240, tracks.tracks, tracks.overrides);
  const report = mockVerifyReport(PATH);

  it("walks detection, edit and leak starts in both directions", () => {
    const forward: number[] = [];
    for (let f: number | null = 0; f !== null; f = nextMark(f, 1, index, report)) forward.push(f);
    const backward: number[] = [];
    for (let f: number | null = 239; f !== null; f = nextMark(f, -1, index, report))
      backward.push(f);
    // Both walks visit the same stops (the Manual box starts on frame 0).
    expect(forward).toEqual(backward.slice(1).toReversed());
    // The open leak (30) and the out-of-scope name (40) are stops, the fixed leak (120) is not.
    expect(forward).toContain(30);
    expect(forward).toContain(40);
    expect(forward).not.toContain(120);
    expect(nextMark(239, 1, index, report)).toBeNull();
  });
});
