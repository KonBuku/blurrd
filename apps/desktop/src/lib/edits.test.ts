import { describe, expect, it } from "vitest";

import type { Override } from "@/bindings/Override";
import type { Track } from "@/bindings/Track";

import {
  addBox,
  adjustTrackBox,
  defaultRange,
  dragRect,
  hideTrackBox,
  MOVE,
  rectFromCorners,
  removeEdit,
  setBoxRect,
  setRange,
  withIn,
  withOut,
  type TrackBoxTarget,
} from "./edits";
import { IntervalIndex } from "./interval-index";

const UID_A = "00000000-0000-4000-8000-00000000000a";
const UID_B = "00000000-0000-4000-8000-00000000000b";

/** One kill feed track at a fixed place over frames 10–50. */
const TRACK: Track = {
  id: 1,
  region: "KillFeed",
  kind: "Victim",
  source: "Detector",
  segments: [{ f0: 10, f1: 50, rect: [100, 20, 160, 32], conf: "Tracked", text_offset: [0, 0] }],
};

const TARGET: TrackBoxTarget = {
  region: "KillFeed",
  span: [10, 50],
  source: [100, 20, 160, 32],
  adjustUid: null,
};

describe("edits", () => {
  it("spans a rectangle from two corners, clipped and snapped", () => {
    expect(rectFromCorners({ x: 30.4, y: 40.6 }, { x: 10.2, y: 5 }, 640, 360)).toEqual([
      10, 5, 30, 41,
    ]);
    expect(rectFromCorners({ x: -5, y: -5 }, { x: 700, y: 400 }, 640, 360)).toEqual([
      0, 0, 640, 360,
    ]);
    expect(rectFromCorners({ x: 10, y: 10 }, { x: 11, y: 40 }, 640, 360)).toBeNull();
  });

  it("moves inside the frame and resizes without collapsing", () => {
    expect(dragRect([10, 10, 50, 30], MOVE, -20, 5.4, 640, 360)).toEqual([0, 15, 40, 35]);
    expect(dragRect([600, 10, 640, 30], MOVE, 25, 0, 640, 360)).toEqual([600, 10, 640, 30]);
    const left = { left: true, top: false, right: false, bottom: false };
    expect(dragRect([10, 10, 50, 30], left, 100, 0, 640, 360)).toEqual([48, 10, 50, 30]);
    const se = { left: false, top: false, right: true, bottom: true };
    expect(dragRect([10, 10, 50, 30], se, 900, 900, 640, 360)).toEqual([10, 10, 640, 360]);
  });

  it("sets in and out points that keep the range ordered", () => {
    expect(defaultRange(30, 60, 240)).toEqual([0, 90]);
    expect(defaultRange(200, 59.94, 240)).toEqual([140, 239]);
    expect(withIn([10, 20], 25)).toEqual([25, 25]);
    expect(withOut([10, 20], 5)).toEqual([5, 5]);
    expect(withOut([10, 20], 30)).toEqual([10, 30]);
  });

  it("adds, moves, re-times and removes manual boxes", () => {
    const added = addBox([], UID_A, [0, 10], [1, 2, 30, 40]);
    expect(added).toEqual([
      { op: "Add", uid: UID_A, region: "Manual", f0: 0, f1: 10, rect: [1, 2, 30, 40] },
    ]);
    const moved = setBoxRect(added, UID_A, [5, 2, 34, 40]);
    expect(moved[0]).toMatchObject({ rect: [5, 2, 34, 40] });
    expect(setBoxRect(moved, UID_A, [5, 2, 34, 40])).toBe(moved);
    expect(setRange(moved, UID_A, [3, 8])[0]).toMatchObject({ f0: 3, f1: 8 });
    expect(removeEdit(moved, UID_A)).toEqual([]);
    expect(removeEdit(moved, UID_B)).toBe(moved);
  });

  it("adjusts a detector box with one edit that the index applies", () => {
    const list = adjustTrackBox([], TARGET, [96, 18, 164, 34], UID_A);
    expect(list).toEqual([
      {
        op: "Adjust",
        uid: UID_A,
        region: "KillFeed",
        f0: 10,
        f1: 50,
        anchor: [100, 20, 160, 32],
        rect: [96, 18, 164, 34],
      },
    ]);
    const index = new IntervalIndex(60, [TRACK], list);
    const [box] = index.boxesAt(30, new Set(["KillFeed"]));
    expect(box?.rect).toEqual([96, 18, 164, 34]);
    expect(box?.source).toEqual([100, 20, 160, 32]);
    expect(box?.adjustUid).toBe(UID_A);

    // Editing again changes the same edit; back at the detector's box removes it.
    const again = adjustTrackBox(list, { ...TARGET, adjustUid: UID_A }, [90, 18, 164, 34], UID_B);
    expect(again).toHaveLength(1);
    expect(again[0]).toMatchObject({ uid: UID_A, rect: [90, 18, 164, 34] });
    expect(adjustTrackBox(again, { ...TARGET, adjustUid: UID_A }, TARGET.source, UID_B)).toEqual(
      [],
    );
    expect(adjustTrackBox([], TARGET, TARGET.source, UID_B)).toEqual([]);
  });

  it("hides a detector box, also after it was moved", () => {
    const moved = adjustTrackBox([], TARGET, [96, 18, 164, 34], UID_A);
    const hidden: Override[] = hideTrackBox(moved, TARGET, UID_B);
    expect(hidden.at(-1)).toMatchObject({ op: "Suppress", anchor: TARGET.source, f0: 10, f1: 50 });
    const index = new IntervalIndex(60, [TRACK], hidden);
    expect(index.boxesAt(30, new Set(["KillFeed"]))).toEqual([]);
    expect(index.active).toEqual([true, true]);
  });
});
