import { describe, expect, it } from "vitest";

import type { Override } from "@/bindings/Override";

import { MockProjects, mergeRanges, touchedRanges } from "./edits";

const PATH = String.raw`D:\Recordings\clip.mp4`;
const ADD: Override = {
  op: "Add",
  uid: "00000000-0000-4000-8000-000000000001",
  region: "Manual",
  f0: 10,
  f1: 20,
  rect: [1, 1, 9, 9],
};

describe("mock set_overrides", () => {
  it("merges touching ranges and lists the frames of changed edits", () => {
    expect(
      mergeRanges([
        [30, 40],
        [0, 5],
        [6, 8],
        [35, 50],
      ]),
    ).toEqual([
      [0, 8],
      [30, 50],
    ]);
    const moved: Override = { ...ADD, f0: 50, f1: 60 };
    expect(touchedRanges([ADD], [moved])).toEqual([
      [10, 20],
      [50, 60],
    ]);
    expect(touchedRanges([ADD], [ADD])).toEqual([]);
    expect(touchedRanges([], [ADD])).toEqual([[10, 20]]);
  });

  it("stores a new revision, rejects stale ones and bad edits", () => {
    const projects = new MockProjects();
    const { revision, overrides } = projects.state(PATH).set;
    const saved = projects.setOverrides(PATH, revision, [...overrides, ADD]);
    expect(saved.revision).toBe(revision + 1);
    expect(saved.unverified_ranges).toEqual([
      [10, 20],
      [150, 170],
    ]);
    expect(() => projects.setOverrides(PATH, revision, [])).toThrow(
      expect.objectContaining({ kind: "StaleRevision" }),
    );
    expect(() => projects.setOverrides(PATH, saved.revision, [ADD, ADD])).toThrow(
      expect.objectContaining({ kind: "InvalidProject" }),
    );
    expect(() => projects.setOverrides(PATH, saved.revision, [{ ...ADD, f1: 999 }])).toThrow(
      expect.objectContaining({ kind: "InvalidProject" }),
    );
  });
});
