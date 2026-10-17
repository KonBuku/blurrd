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
  /**
   * The pass over the video the running job is in, from 0, and the
   * passes it planned (`Started`). Each pass counts its frames from 0
   * and fills an equal share of the job; a run of an edited file
   * analyses, then renders (two passes).
   */
  readonly pass: number;
  readonly passes: number;
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
    pass: 0,
    passes: 1,
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
    pass: 0,
    passes: 1,
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
        passes: Math.max(event.passes, 1),
      };
    }
    case "Stage": {
      const planned = new Set(progress.plan.flatMap((kind) => JOB_STAGES[kind]));
      const extra =
        planned.has(event.stage) || progress.extra.includes(event.stage)
          ? progress.extra
          : [...progress.extra, event.stage];
      // A new pass counts its frames from 0 again.
      const restart = event.pass > progress.pass;
      return {
        ...progress,
        step,
        stage: event.stage,
        extra,
        pass: Math.max(event.pass, progress.pass),
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
 * Completion of the running job by its frames; `null` while unknown. A
 * pass beyond the planned ones (a run rendering again after its
 * verification added fix boxes) shares the job with the passes before
 * it; `jobShown` keeps the bar from going back.
 */
function jobFraction(progress: OperationProgress): number | null {
  if (progress.total <= 0) return null;
  const frames = Math.min(Math.max(progress.frame / progress.total, 0), 1);
  return (progress.pass + frames) / Math.max(progress.passes, progress.pass + 1);
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
