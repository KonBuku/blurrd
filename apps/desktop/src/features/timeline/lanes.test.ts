import { describe, expect, it } from "vitest";

import type { VerifyReport } from "@/bindings/VerifyReport";
import { mockVerifyReport } from "@/ipc/mock/project";

import { lanePath } from "./DensityLanes";
import { verifySummary } from "./VerifyLane";

describe("lanePath", () => {
  it("draws the largest coverage per pixel column, at least 2 px", () => {
    const coverage = [0, 0, 0.01, 1, 0.5, 0, 0, 0];
    // 8 buckets of 10 frames on 8 px: one column per bucket.
    expect(lanePath(coverage, 80, { start: 0, end: 80 }, 8, 10)).toBe(
      "M2 8h1v2h-1zM3 0h1v10h-1zM4 5h1v5h-1z",
    );
    // 2 px for 8 buckets: each column takes the maximum of four.
    expect(lanePath(coverage, 80, { start: 0, end: 80 }, 2, 10)).toBe("M0 0h1v10h-1zM1 5h1v5h-1z");
    expect(lanePath([0, 0], 20, { start: 0, end: 20 }, 4, 10)).toBe("");
  });
});

describe("verifySummary", () => {
  it("sums up the Verify lane", () => {
    expect(verifySummary(null, [])).toBe("Not verified yet.");
    const report = mockVerifyReport(String.raw`D:\a.mp4`) as VerifyReport;
    expect(verifySummary(report, [[150, 170]])).toBe(
      "1 open leak, 1 fixed, frames 150–170 not verified since the last edit.",
    );
  });
});
