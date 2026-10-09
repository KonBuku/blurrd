import { describe, expect, it } from "vitest";

import type { Override } from "@/bindings/Override";
import type { Region } from "@/bindings/Region";
import type { Track } from "@/bindings/Track";

import { IntervalIndex, iou, trackSeed, uidSeed } from "./interval-index";

const TRACKS: Track[] = [
  {
    id: 1,
    region: "KillFeed",
    kind: "Killer",
    source: "Detector",
    segments: [
      { f0: 0, f1: 9, rect: [10, 10, 60, 22], conf: "PreRoll", text_offset: [0, 0] },
      { f0: 10, f1: 49, rect: [12, 10, 62, 22], conf: "Tracked", text_offset: [2, 0] },
    ],
  },
  {
    id: 7,
    region: "Banner",
    kind: "Banner",
    source: "Detector",
    segments: [
      { f0: 20, f1: 60, rect: [200, 100, 300, 120], conf: "Tracked", text_offset: [0, 0] },
    ],
  },
  {
    id: 3,
    region: "VoiceChat",
    kind: "Speaker",
    source: "OcrFix",
    segments: [{ f0: 30, f1: 40, rect: [5, 300, 80, 312], conf: "OcrFix", text_offset: [-1, 1] }],
  },
];

const OVERRIDES: Override[] = [
  {
    op: "Suppress",
    uid: "11111111-2222-4333-8444-555555555555",
    region: "Banner",
    f0: 25,
    f1: 30,
    anchor: [201, 100, 300, 121],
  },
  {
    op: "Adjust",
    uid: "21111111-2222-4333-8444-555555555555",
    region: "KillFeed",
    f0: 40,
    f1: 45,
    anchor: [12, 10, 62, 22],
    rect: [0, 0, 70, 30],
  },
  {
    op: "Add",
    uid: "0f8c3a52-6d0e-4b8e-9c1a-2a5d7b3e9f10",
    region: "Manual",
    f0: 0,
    f1: 99,
    rect: [300, 241, 341, 263],
  },
  {
    op: "Suppress",
    uid: "31111111-2222-4333-8444-555555555555",
    region: "KillFeed",
    f0: 0,
    f1: 99,
    anchor: [400, 400, 420, 410],
  },
];

const ALL = new Set<Region>(["KillFeed", "Banner", "VoiceChat", "TextChat"]);

/** Seeds printed by `BlurPlan::boxes_at` for the same plan. */
const SEED = { killfeed: 4255298252, banner: 2231509539, voice: 3049962703, manual: 873165834 };

/** Boxes of a frame as `region rect seed grid scale`, like the Rust printout. */
function describeFrame(index: IntervalIndex, frame: number, enabled = ALL): string[] {
  return index
    .boxesAt(frame, enabled)
    .map(
      (b) => `${b.region} ${b.rect.join(",")} ${b.seed} ${b.gridOrigin.join(",")} ${b.blockScale}`,
    );
}

describe("IntervalIndex", () => {
  const index = new IntervalIndex(100, TRACKS, OVERRIDES);

  it("derives the same seeds as the engine", () => {
    expect(trackSeed(1, "KillFeed")).toBe(SEED.killfeed);
    expect(trackSeed(7, "Banner")).toBe(SEED.banner);
    expect(trackSeed(3, "VoiceChat")).toBe(SEED.voice);
    expect(uidSeed("0f8c3a52-6d0e-4b8e-9c1a-2a5d7b3e9f10")).toBe(SEED.manual);
  });

  it("binds edits like the engine", () => {
    expect(index.active).toEqual([true, true, true, false]);
  });

  it("returns the boxes of BlurPlan::boxes_at", () => {
    const manual = `Manual 300,241,341,263 ${SEED.manual} 300,241 1`;
    expect(describeFrame(index, 5)).toEqual([
      `KillFeed 10,10,60,22 ${SEED.killfeed} 10,10 1.5`,
      manual,
    ]);
    // The banner is hidden on frames 25..30.
    expect(describeFrame(index, 27)).toEqual([
      `KillFeed 12,10,62,22 ${SEED.killfeed} 12,10 1`,
      manual,
    ]);
    expect(describeFrame(index, 35)).toEqual([
      `KillFeed 12,10,62,22 ${SEED.killfeed} 12,10 1`,
      `Banner 200,100,300,120 ${SEED.banner} 200,100 1`,
      `VoiceChat 5,300,80,312 ${SEED.voice} 4,301 1.5`,
      manual,
    ]);
    // The adjusted kill-feed box keeps the grid of the text.
    expect(describeFrame(index, 42)).toEqual([
      `KillFeed 0,0,70,30 ${SEED.killfeed} 12,10 1`,
      `Banner 200,100,300,120 ${SEED.banner} 200,100 1`,
      manual,
    ]);
    expect(describeFrame(index, 55)).toEqual([
      `Banner 200,100,300,120 ${SEED.banner} 200,100 1`,
      manual,
    ]);
  });

  it("skips disabled regions but never Manual boxes", () => {
    expect(describeFrame(index, 35, new Set<Region>(["Banner"]))).toEqual([
      `Banner 200,100,300,120 ${SEED.banner} 200,100 1`,
      `Manual 300,241,341,263 ${SEED.manual} 300,241 1`,
    ]);
    expect(index.boxesAt(100, ALL)).toEqual([]);
    expect(index.boxesAt(-1, ALL)).toEqual([]);
  });

  it("reports what each box is for the overlay", () => {
    const [feed, , , manual] = index.boxesAt(35, ALL);
    expect(feed).toMatchObject({ trackId: 1, uid: null, kind: "Killer", span: [0, 49] });
    expect(manual).toMatchObject({ trackId: null, kind: "Manual", span: [0, 99] });
  });

  it("finds the next and previous detection start", () => {
    expect(index.nextStart(0, 1)).toBe(10);
    expect(index.nextStart(25, 1)).toBe(30);
    expect(index.nextStart(25, -1)).toBe(20);
    expect(index.nextStart(0, -1)).toBeNull();
    expect(index.nextStart(30, 1)).toBeNull();
  });

  it("uses buckets that cover long videos", () => {
    const long: Track = {
      ...TRACKS[1],
      id: 9,
      segments: [{ f0: 700, f1: 1300, rect: [1, 1, 5, 5], conf: "Tracked", text_offset: [0, 0] }],
    } as Track;
    const big = new IntervalIndex(2000, [long], []);
    expect(big.boxesAt(699, ALL)).toHaveLength(0);
    expect(big.boxesAt(700, ALL)).toHaveLength(1);
    expect(big.boxesAt(1024, ALL)).toHaveLength(1);
    expect(big.boxesAt(1301, ALL)).toHaveLength(0);
  });
});

describe("iou", () => {
  it("is the overlap over the union", () => {
    expect(iou([0, 0, 10, 10], [5, 0, 15, 10])).toBeCloseTo(1 / 3, 6);
    expect(iou([0, 0, 10, 10], [10, 0, 20, 10])).toBe(0);
  });
});
