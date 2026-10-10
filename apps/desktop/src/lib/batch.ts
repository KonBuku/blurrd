/**
 * A batch: the operations started while others were still running, from
 * the first start until the last one ends. The queue footer and the
 * taskbar show the batch's overall progress; when it ends, one system
 * notification sums it up (design 8.3).
 */
import { formatCount } from "./media";
import { operationFraction, type Operation, type OperationProgress } from "./job-progress";

/** How an operation of the batch ended. */
export interface Outcome {
  readonly name: string;
  readonly operation: Operation;
  readonly result: "done" | "failed" | "canceled";
  /**
   * For `done`: the result in a few words ("verified clean", "review the
   * report"); for `failed`: the error message.
   */
  readonly text: string;
  /** Whether the result needs the user (possible leaks, unverified output). */
  readonly review: boolean;
  /** The written file, for a finished export. */
  readonly output: string | null;
}

/** The operations of a batch: how many were started and how the ended ones ended. */
export interface Batch {
  readonly size: number;
  /** Outcomes of the ended operations, in the order they ended. */
  readonly ended: readonly Outcome[];
}

/**
 * Completion of the batch from 0 to 1: ended operations count as whole,
 * running ones by their own completion (waiting ones as 0).
 */
export function batchFraction(
  batch: Batch,
  running: Readonly<Record<string, OperationProgress>>,
): number {
  if (batch.size <= 0) return 0;
  const done = batch.ended.length;
  const partial = Object.values(running).reduce((sum, progress) => {
    const fraction =
      operationFraction(progress) ?? Math.max(progress.step, 0) / progress.plan.length;
    return sum + fraction;
  }, 0);
  return Math.min(Math.max((done + partial) / batch.size, 0), 1);
}

/** The state of the taskbar button's progress (Tauri `ProgressBarStatus`). */
export type TaskbarStatus = "none" | "normal" | "paused" | "error";

/** What the taskbar button shows: a status and a whole percentage. */
export interface TaskbarState {
  readonly status: TaskbarStatus;
  readonly progress: number;
}

/** No progress on the taskbar button. */
export const TASKBAR_IDLE: TaskbarState = { status: "none", progress: 0 };

/**
 * The taskbar progress of a batch: nothing without one, paused while the
 * queue is paused, the error colour once an operation failed, otherwise
 * the normal colour, always with the batch's overall percentage.
 */
export function taskbarState(
  batch: Batch | null,
  running: Readonly<Record<string, OperationProgress>>,
  paused: boolean,
): TaskbarState {
  if (batch === null) return TASKBAR_IDLE;
  const progress = Math.floor(batchFraction(batch, running) * 100);
  const failed = batch.ended.some((outcome) => outcome.result === "failed");
  return { status: paused ? "paused" : failed ? "error" : "normal", progress };
}

/** Title and text of the notification that sums up an ended batch. */
export interface BatchNotice {
  readonly title: string;
  readonly body: string;
}

/** The file name without its extension, as notifications name files. */
function stem(name: string): string {
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(0, dot) : name;
}

/** "Export", "Analysis" or "Processing" for the operations of a batch. */
function batchNoun(operations: ReadonlySet<Operation>): string {
  if (operations.size !== 1) return "Processing";
  return operations.has("export") ? "Export" : "Analysis";
}

/**
 * The notification of an ended batch: one file is named with its result
 * ("match_bind_02 exported · verified clean"), several are counted
 * ("3 exported · 1 needs review · 1 failed"). A batch whose operations
 * were all canceled needs no notification (`null`): the user ended it.
 */
export function batchNotice(batch: Batch): BatchNotice | null {
  const relevant = batch.ended.filter((outcome) => outcome.result !== "canceled");
  if (relevant.length === 0) return null;
  const failed = relevant.filter((outcome) => outcome.result === "failed");
  const noun = batchNoun(new Set(relevant.map((outcome) => outcome.operation)));
  const title =
    failed.length === relevant.length
      ? `${noun} failed`
      : failed.length > 0
        ? `${noun} finished with errors`
        : `${noun} finished`;

  const only = relevant.length === 1 ? relevant[0] : undefined;
  if (only) {
    const verb = only.operation === "export" ? "exported" : "analyzed";
    const body =
      only.result === "failed"
        ? `${stem(only.name)}: ${only.text}`
        : [`${stem(only.name)} ${verb}`, only.text].filter((part) => part !== "").join(" · ");
    return { title, body };
  }

  const done = relevant.filter((outcome) => outcome.result === "done");
  const count = (operation: Operation) =>
    done.filter((outcome) => outcome.operation === operation).length;
  const review = done.filter((outcome) => outcome.review).length;
  const parts = [
    count("export") > 0 ? `${formatCount(count("export"))} exported` : null,
    count("analyze") > 0 ? `${formatCount(count("analyze"))} analyzed` : null,
    review > 0 ? `${formatCount(review)} ${review === 1 ? "needs" : "need"} review` : null,
    failed.length > 0 ? `${formatCount(failed.length)} failed` : null,
  ];
  return { title, body: parts.filter((part): part is string => part !== null).join(" · ") };
}
