import { describe, expect, it } from "vitest";

import {
  isFinished,
  isRunning,
  statusProgress,
  statusRank,
  statusText,
  statusTone,
  type QueueStatus,
} from "./queue-status";

/** One status of every kind, with the text and tone design 8.1 asks for. */
const CASES: readonly [QueueStatus, string, string][] = [
  [{ kind: "queued" }, "Queued", "muted"],
  [{ kind: "probing", folder: false }, "Reading file…", "neutral"],
  [{ kind: "probing", folder: true }, "Reading folder…", "neutral"],
  [{ kind: "analyzing", progress: 0.462, fps: 820.4 }, "Analyzing · 46% · 820 fps", "neutral"],
  [{ kind: "analyzing", progress: null, fps: null }, "Analyzing", "neutral"],
  [{ kind: "ready", boxes: 1284 }, "Ready · 1,284 boxes", "neutral"],
  [{ kind: "ready", boxes: 1 }, "Ready · 1 box", "neutral"],
  [{ kind: "ready", boxes: null }, "Ready", "neutral"],
  [
    { kind: "exporting", progress: 0.63, fps: 388, etaMs: 21_000 },
    "Exporting · 63% · 388 fps · 0:21 left",
    "neutral",
  ],
  [{ kind: "verifying", pass: 2, passes: 3 }, "Verifying · pass 2/3", "neutral"],
  [{ kind: "done", verified: true }, "Done · verified clean", "success"],
  [{ kind: "done", verified: false }, "Done · not verified", "neutral"],
  [{ kind: "warnings", leaks: 2 }, "Done · 2 possible leaks", "warning"],
  [{ kind: "warnings", leaks: 1 }, "Done · 1 possible leak", "warning"],
  [{ kind: "unverified" }, "Unverified · review the report", "warning"],
  [{ kind: "failed", reason: "Decoder error", detail: null }, "Decoder error", "destructive"],
  [{ kind: "canceled" }, "Canceled", "muted"],
];

describe("queue status", () => {
  it.each(CASES)("describes %o", (status, text, tone) => {
    expect(statusText(status)).toBe(text);
    expect(statusTone(status)).toBe(tone);
  });

  it("clamps progress in the text", () => {
    expect(statusText({ kind: "analyzing", progress: 1.4, fps: null })).toBe("Analyzing · 100%");
  });

  it("shows a bar only while a job runs", () => {
    expect(statusProgress({ kind: "probing", folder: false })).toBeNull();
    expect(statusProgress({ kind: "analyzing", progress: 0.5, fps: null })).toBe(0.5);
    expect(statusProgress({ kind: "verifying", pass: 2, passes: 4 })).toBe(0.25);
    expect(statusProgress({ kind: "ready", boxes: 3 })).toBeUndefined();
    expect(statusProgress({ kind: "unverified" })).toBeUndefined();
  });

  it("separates running, finished and waiting files", () => {
    const running = CASES.filter(([status]) => isRunning(status)).map(([status]) => status.kind);
    const finished = CASES.filter(([status]) => isFinished(status)).map(([status]) => status.kind);
    expect(new Set(running)).toEqual(new Set(["probing", "analyzing", "exporting", "verifying"]));
    expect(new Set(finished)).toEqual(
      new Set(["done", "warnings", "unverified", "failed", "canceled"]),
    );
  });

  it("sorts what needs attention first", () => {
    const ranks = CASES.map(([status]) => statusRank(status));
    expect(Math.min(...ranks)).toBe(statusRank({ kind: "failed", reason: "", detail: null }));
    expect(statusRank({ kind: "unverified" })).toBeLessThan(statusRank({ kind: "queued" }));
  });
});
