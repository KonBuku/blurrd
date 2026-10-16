/**
 * Progress of an operation the user started for one file: an analysis,
 * or an export made of one or more backend jobs (a re-verification of
 * edited ranges, then the render; or one job of the run queue). The
 * processing card, the queue status, the queue footer and the taskbar
 * all read it; `applyJobEvent` folds the events of the running job in.
 */
import type { JobEvent } from "@/bindings/JobEvent";
import type { JobKind } from "@/bindings/JobKind";
import type { Stage } from "@/bindings/Stage";

import { JOB_STAGES, stepsOf } from "./job-stages";

/** What the user asked for. */
export type Operation = "analyze" | "export";

/**
 * How the frames of the running job count. A run job passes over the
 * video once (analysis and render together) or twice (an analysis, then
 * the render of a project with edits), counting frames from 0 each time;
 * which one is known only when the render starts. Until then its frames
 * fill the first half of the job (`first`); a render that starts after
 * that pass fills the second half (`second`), otherwise the frames count
 * for the whole job (`single`), as for every other kind of job.
 */
export type Pass = "single" | "first" | "second";

/** Stages of the render, which a run job's second pass reports. */
const RENDER_STAGES: ReadonlySet<Stage> = new Set(["BlurEncode", "Mux"]);

/** Where an operation stands. */
export interface OperationProgress {
  readonly operation: Operation;
  /** The backend jobs of the operation, in order. */
  readonly plan: readonly JobKind[];
  /** Index of the running job in `plan`; -1 while waiting in the run queue. */
  readonly step: number;
  /** The stage the running job reported last. */
  readonly stage: Stage | null;
  /** Stages reported that the plan did not list (shown as extra steps). */
  readonly extra: readonly Stage[];
  /** Frames done and in total of the running job (`total` 0 = unknown). */
  readonly frame: number;
  readonly total: number;
  readonly fps: number | null;
  readonly etaMs: number | null;
  /** How the running job's frames count. */
  readonly pass: Pass;
  /** Completion of the running job shown so far (it never goes back). */
  readonly jobShown: number;
  /** Warnings reported so far, newest last. */
  readonly warnings: readonly string[];
}

/** An operation of `plan` that has not started yet. */
export function waitingOperation(
  operation: Operation,
  plan: readonly JobKind[],
): OperationProgress {
  return {
    operation,
    plan,
    step: -1,
    stage: null,
    extra: [],
    frame: 0,
    total: 0,
    fps: null,
    etaMs: null,
    pass: "single",
    jobShown: 0,
    warnings: [],
  };
}

/** Moves an operation on to job `step` of its plan, with fresh counters. */
export function startStep(progress: OperationProgress, step: number): OperationProgress {
  return {
    ...progress,
    step,
    stage: null,
    frame: 0,
    total: 0,
    fps: null,
    etaMs: null,
    pass: "single",
    jobShown: 0,
  };
}

/**
 * Folds one event of the running job into the progress. `Started` may
 * name another kind than planned (the run queue renders a file whose
 * analysis is current without analysing it again); the plan follows the
 * backend. End events (`Finished`, `Failed`, `Canceled`) leave the
 * progress as it is; the caller ends the operation.
 */
export function applyJobEvent(progress: OperationProgress, event: JobEvent): OperationProgress {
  const step = Math.max(progress.step, 0);
  switch (event.type) {
    case "Started": {
      const plan = progress.plan.map((kind, index) => (index === step ? event.kind : kind));
      return {
        ...startStep({ ...progress, plan }, step),
        total: event.total_frames,
        pass: event.kind === "Run" ? "first" : "single",
      };
    }
    case "Stage": {
      const planned = new Set(progress.plan.flatMap((kind) => JOB_STAGES[kind]));
      const extra =
        planned.has(event.stage) || progress.extra.includes(event.stage)
          ? progress.extra
          : [...progress.extra, event.stage];
      const pass = passAt(progress, event.stage);
      // The second pass counts its frames from 0 again.
      const restart = pass === "second" && progress.pass !== "second";
      return {
        ...progress,
        step,
        stage: event.stage,
        extra,
        pass,
        ...(restart ? { frame: 0, fps: null, etaMs: null } : {}),
      };
    }
    case "Progress": {
      const next = {
        ...progress,
        step,
        frame: event.frame,
        total: event.total,
        fps: event.fps,
        etaMs: event.eta_ms,
      };
      return { ...next, jobShown: Math.max(progress.jobShown, jobFraction(next) ?? 0) };
    }
    case "Warning":
      return { ...progress, warnings: [...progress.warnings, event.message] };
    default:
      return progress;
  }
}

/**
 * The pass of a run job once it reports `stage`: a render that starts
 * after at least half of the frames were analysed is the second pass.
 */
function passAt(progress: OperationProgress, stage: Stage): Pass {
  if (progress.pass !== "first" || !RENDER_STAGES.has(stage)) return progress.pass;
  return progress.total > 0 && progress.frame * 2 >= progress.total ? "second" : "single";
}

/** Completion of the running job by its frames; `null` while unknown. */
function jobFraction(progress: OperationProgress): number | null {
  if (progress.total <= 0) return null;
  const frames = Math.min(Math.max(progress.frame / progress.total, 0), 1);
  switch (progress.pass) {
    case "first":
      return frames / 2;
    case "second":
      return 0.5 + frames / 2;
    default:
      return frames;
  }
}

/** The steps of the processing card's stepper. */
export function operationSteps(progress: OperationProgress): Stage[] {
  return stepsOf(progress.plan, progress.extra);
}

/**
 * Completion of the whole operation from 0 to 1: finished jobs count as
 * whole, the running one by its frames (both passes of a run job
 * together, so the bar never goes back). `null` while the running job
 * has not said how many frames it will process.
 */
export function operationFraction(progress: OperationProgress): number | null {
  if (progress.step < 0) return 0;
  const job = jobFraction(progress);
  if (job === null) {
    if (progress.frame > 0) return null;
    return (progress.step + progress.jobShown) / progress.plan.length;
  }
  return (progress.step + Math.max(job, progress.jobShown)) / progress.plan.length;
}
