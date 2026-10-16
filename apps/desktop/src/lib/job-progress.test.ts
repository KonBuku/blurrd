import { describe, expect, it } from "vitest";

import type { JobEvent } from "@/bindings/JobEvent";

import { batchFraction, batchNotice, taskbarState, type Batch, type Outcome } from "./batch";
import {
  applyJobEvent,
  operationFraction,
  operationSteps,
  startStep,
  waitingOperation,
} from "./job-progress";
import { stepsOf, stepStates } from "./job-stages";

const JOB = "job-1";

/** Folds `events` into a fresh operation of `plan`. */
function fold(plan: Parameters<typeof waitingOperation>[1], events: JobEvent[]) {
  return events.reduce(applyJobEvent, startStep(waitingOperation("export", plan), 0));
}

describe("job stages", () => {
  it("joins the stages of several jobs in pipeline order", () => {
    expect(stepsOf(["ReverifyRanges", "Render"])).toEqual([
      "Decode",
      "Ocr",
      "Verify",
      "BlurEncode",
      "Mux",
    ]);
    expect(stepsOf(["Render"], ["Probe"])).toEqual(["Probe", "BlurEncode", "Mux"]);
  });

  it("marks steps before the running stage done and later ones pending", () => {
    const steps = stepsOf(["Analyze"]);
    expect(stepStates(steps, "Track")).toEqual(["done", "done", "current", "pending", "pending"]);
    expect(stepStates(steps, null)).toEqual(Array(5).fill("pending"));
  });
});

describe("operation progress", () => {
  it("follows stages, frames, speed and warnings of the running job", () => {
    const progress = fold(
      ["Render"],
      [
        { type: "Started", job: JOB, kind: "Render", total_frames: 9060 },
        { type: "Stage", job: JOB, stage: "BlurEncode" },
        { type: "Progress", job: JOB, frame: 5712, total: 9060, fps: 388, eta_ms: 21_000 },
        { type: "Warning", job: JOB, code: "EncoderFallback", message: "1 session", frames: null },
      ],
    );
    expect(progress).toMatchObject({
      stage: "BlurEncode",
      frame: 5712,
      total: 9060,
      fps: 388,
      etaMs: 21_000,
      warnings: ["1 session"],
    });
    expect(operationFraction(progress)).toBeCloseTo(5712 / 9060);
  });

  it("counts finished jobs of the plan as whole", () => {
    const second = fold(
      ["ReverifyRanges", "Render"],
      [{ type: "Started", job: JOB, kind: "ReverifyRanges", total_frames: 21 }],
    );
    const rendering = applyJobEvent(startStep(second, 1), {
      type: "Progress",
      job: JOB,
      frame: 50,
      total: 100,
      fps: 400,
      eta_ms: 125,
    });
    expect(operationFraction(rendering)).toBeCloseTo(0.75);
    expect(operationFraction(waitingOperation("export", ["Run"]))).toBe(0);
  });

  it("follows the backend when a run job only renders", () => {
    const progress = fold(
      ["Run"],
      [{ type: "Started", job: JOB, kind: "Render", total_frames: 9 }],
    );
    expect(progress.plan).toEqual(["Render"]);
    expect(operationSteps(progress)).toEqual(["BlurEncode", "Mux"]);
  });

  it("keeps the bar of a run job going forward over both of its passes", () => {
    const progress = (frame: number, total = 200): JobEvent => ({
      type: "Progress",
      job: JOB,
      frame,
      total,
      fps: 100,
      eta_ms: 1000,
    });
    const shown: number[] = [];
    const events: JobEvent[] = [
      { type: "Started", job: JOB, kind: "Run", total_frames: 200 },
      { type: "Stage", job: JOB, stage: "Detect" },
      progress(50),
      progress(200),
      // The render of the edited project counts frames from 0 again.
      { type: "Stage", job: JOB, stage: "Probe" },
      { type: "Stage", job: JOB, stage: "BlurEncode" },
      progress(0),
      progress(100),
      progress(200),
    ];
    events.reduce(
      (state, event) => {
        const next = applyJobEvent(state, event);
        shown.push(operationFraction(next) ?? -1);
        return next;
      },
      startStep(waitingOperation("export", ["Run"]), 0),
    );
    expect(shown).toEqual([0, 0, 0.125, 0.5, 0.5, 0.5, 0.5, 0.75, 1]);

    // One pass (analysis and render streamed together): the render
    // starts early and the frames count for the whole job.
    const streamed = fold(
      ["Run"],
      [
        { type: "Started", job: JOB, kind: "Run", total_frames: 200 },
        progress(20),
        { type: "Stage", job: JOB, stage: "BlurEncode" },
        progress(10),
      ],
    );
    expect(operationFraction(streamed)).toBeCloseTo(0.05);
    expect(operationFraction(applyJobEvent(streamed, progress(150)))).toBeCloseTo(0.75);
  });

  it("adds a stage the plan did not list as an extra step", () => {
    const progress = fold(["Render"], [{ type: "Stage", job: JOB, stage: "Probe" }]);
    expect(operationSteps(progress)).toEqual(["Probe", "BlurEncode", "Mux"]);
  });
});

/** An outcome of a file named `name`. */
function outcome(name: string, change: Partial<Outcome> = {}): Outcome {
  return {
    name,
    operation: "export",
    result: "done",
    text: "verified clean",
    review: false,
    output: `E:\\Exports\\${name}`,
    ...change,
  };
}

describe("batch", () => {
  const running = fold(
    ["Render"],
    [{ type: "Progress", job: JOB, frame: 50, total: 100, fps: 1, eta_ms: 1 }],
  );

  it("adds ended and running operations into one fraction", () => {
    const batch: Batch = { size: 4, ended: [outcome("a.mp4")] };
    const waiting = waitingOperation("export", ["Run"]);
    expect(batchFraction(batch, { b: running, c: waiting, d: waiting })).toBeCloseTo(1.5 / 4);
  });

  it("drives the taskbar: idle, normal, paused and error", () => {
    expect(taskbarState(null, {}, false)).toEqual({ status: "none", progress: 0 });
    const batch: Batch = { size: 2, ended: [outcome("a.mp4")] };
    expect(taskbarState(batch, { b: running }, false)).toEqual({ status: "normal", progress: 75 });
    expect(taskbarState(batch, { b: running }, true).status).toBe("paused");
    const failed: Batch = { size: 2, ended: [outcome("a.mp4", { result: "failed" })] };
    expect(taskbarState(failed, { b: running }, false).status).toBe("error");
  });

  it("names one file with its result", () => {
    expect(batchNotice({ size: 1, ended: [outcome("match_bind_02.mp4")] })).toEqual({
      title: "Export finished",
      body: "match_bind_02 exported · verified clean",
    });
    const failed = outcome("match_bind_02.mp4", { result: "failed", text: "Disk full." });
    expect(batchNotice({ size: 1, ended: [failed] })).toEqual({
      title: "Export failed",
      body: "match_bind_02: Disk full.",
    });
    const analyzed = outcome("clip.mp4", { operation: "analyze", text: "", output: null });
    expect(batchNotice({ size: 1, ended: [analyzed] })).toEqual({
      title: "Analysis finished",
      body: "clip analyzed",
    });
  });

  it("counts several files and skips a batch the user canceled", () => {
    const canceled = outcome("d.mp4", { result: "canceled" });
    const ended = [
      outcome("a.mp4"),
      outcome("b.mp4", { text: "review the report", review: true }),
      outcome("c.mp4", { result: "failed", text: "Disk full." }),
      canceled,
    ];
    expect(batchNotice({ size: 4, ended })).toEqual({
      title: "Export finished with errors",
      body: "2 exported · 1 needs review · 1 failed",
    });
    expect(batchNotice({ size: 1, ended: [canceled] })).toBeNull();
  });
});
