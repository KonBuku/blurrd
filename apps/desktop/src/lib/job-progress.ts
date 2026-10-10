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
    warnings: [],
  };
}

/** Moves an operation on to job `step` of its plan, with fresh counters. */
export function startStep(progress: OperationProgress, step: number): OperationProgress {
  return { ...progress, step, stage: null, frame: 0, total: 0, fps: null, etaMs: null };
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
      return { ...startStep({ ...progress, plan }, step), total: event.total_frames };
    }
    case "Stage": {
      const planned = new Set(progress.plan.flatMap((kind) => JOB_STAGES[kind]));
      const extra =
        planned.has(event.stage) || progress.extra.includes(event.stage)
          ? progress.extra
          : [...progress.extra, event.stage];
      return { ...progress, step, stage: event.stage, extra };
    }
    case "Progress":
      return {
        ...progress,
        step,
        frame: event.frame,
        total: event.total,
        fps: event.fps,
        etaMs: event.eta_ms,
      };
    case "Warning":
      return { ...progress, warnings: [...progress.warnings, event.message] };
    default:
      return progress;
  }
}

/** The steps of the processing card's stepper. */
export function operationSteps(progress: OperationProgress): Stage[] {
  return stepsOf(progress.plan, progress.extra);
}

/**
 * Completion of the whole operation from 0 to 1: finished jobs count as
 * whole, the running one by its frames. `null` while the running job
 * has not said how many frames it will process.
 */
export function operationFraction(progress: OperationProgress): number | null {
  if (progress.step < 0) return 0;
  if (progress.total <= 0) return progress.frame > 0 ? null : progress.step / progress.plan.length;
  const job = Math.min(Math.max(progress.frame / progress.total, 0), 1);
  return (progress.step + job) / progress.plan.length;
}
