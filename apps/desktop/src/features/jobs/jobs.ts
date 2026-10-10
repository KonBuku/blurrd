/**
 * Runs the operations of queued files and mirrors their job events in the
 * queue status (design 8.1 status model) and the processing card (design
 * 8.3): progress while running, then the resting status, a failure with
 * its message and details, and a toast. Operations started while others
 * run form a batch; when the batch ends, one system notification sums it
 * up if the window is in the background, and the output folder opens when
 * the user asked for that (Settings › General; see `batch-end.ts`).
 *
 * An export of one file verifies the ranges that edits changed first
 * (`start_reverify`, plan 7), then renders. "Export N files" hands the
 * files to the backend's run queue (`start_run`), so the order and the
 * hand-over from one file to the next never wait for the webview.
 */
import type { QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { create } from "zustand";

import type { JobEvent } from "@/bindings/JobEvent";
import type { JobId } from "@/bindings/JobId";
import type { JobKind } from "@/bindings/JobKind";
import type { JobSummary } from "@/bindings/JobSummary";
import { toAppError } from "@/ipc/errors";
import {
  cancelJob,
  pauseQueue,
  resumeQueue,
  startAnalysis,
  startRender,
  startReverify,
  startRun,
  type JobListener,
} from "@/ipc/jobs";
import type { Batch, Outcome } from "@/lib/batch";
import { analysisRequest, renderRequest, runFile } from "@/lib/job-request";
import {
  applyJobEvent,
  operationFraction,
  startStep,
  waitingOperation,
  type Operation,
  type OperationProgress,
} from "@/lib/job-progress";
import type { JobSettings } from "@/lib/job-settings";
import { logWarning } from "@/lib/log";
import { baseName } from "@/lib/media";
import { restingStatus, type QueueStatus } from "@/lib/queue-status";
import { useQueueStore, type QueueItem } from "@/store/queue";

import { batchEnded, showInFolder } from "./batch-end";
import { showFailureDetails } from "./FailureDetailsDialog";

/** An operation in flight for a queued file. */
export interface RunningOperation {
  readonly name: string;
  /** The backend's id of the running (or waiting) job, once known. */
  readonly job: JobId | null;
  /**
   * The user canceled before the backend answered the job id; the job is
   * canceled as soon as the id arrives, and no further job starts.
   */
  readonly cancelRequested: boolean;
  readonly progress: OperationProgress;
}

/** A file an export wrote in this session (Empty state › Recent exports). */
export interface RecentExport {
  readonly path: string;
  /** The result in a few words ("verified clean"). */
  readonly text: string;
}

/** How many recent exports are kept. */
export const RECENT_EXPORTS = 5;

/** Operations in flight, the batch they form and what this session exported. */
export interface JobsState {
  /** Operations by queue item id (one per file at a time). */
  readonly running: Readonly<Record<string, RunningOperation>>;
  /** Whether the backend's jobs are paused (`pause_queue`). */
  readonly paused: boolean;
  /** The current batch, `null` while nothing runs. */
  readonly batch: Batch | null;
  /** Files exported in this session, newest first. */
  readonly recent: readonly RecentExport[];
}

export const useJobsStore = create<JobsState>()(() => ({
  running: {},
  paused: false,
  batch: null,
  recent: [],
}));

/** The file an operation runs for. */
export type JobItem = Pick<QueueItem, "id" | "path" | "name" | "status">;

/** One backend job of an operation. */
interface StepSpec {
  readonly kind: JobKind;
  /** Starts the job; `null` for a job the run queue started already. */
  readonly start: ((onEvent: JobListener) => Promise<JobId>) | null;
}

/** What differs between operations. */
interface OperationSpec {
  readonly operation: Operation;
  readonly steps: readonly StepSpec[];
  /** The resting status after the last job finished. */
  readonly finished: (summary: JobSummary) => QueueStatus;
  /** Runs after the last job finished (reloading what the operation changed). */
  readonly after: () => void;
}

/** "analyzed" or "exported", for the messages. */
function verbOf(operation: Operation): string {
  return operation === "export" ? "exported" : "analyzed";
}

/** The queue status while an operation runs. */
function runningStatus(progress: OperationProgress): QueueStatus {
  const fraction = operationFraction(progress);
  return progress.operation === "export"
    ? { kind: "exporting", progress: fraction, fps: progress.fps, etaMs: progress.etaMs }
    : { kind: "analyzing", progress: fraction, fps: progress.fps };
}

/** The result of a finished operation in a few words, and whether it needs the user. */
export function resultOf(status: QueueStatus): { readonly text: string; readonly review: boolean } {
  switch (status.kind) {
    case "done":
      return { text: status.verified ? "verified clean" : "not verified", review: false };
    case "warnings":
      return {
        text: `${status.leaks} possible ${status.leaks === 1 ? "leak" : "leaks"}`,
        review: true,
      };
    case "unverified":
      return { text: "review the report", review: true };
    default:
      return { text: "", review: false };
  }
}

function patchRunning(id: string, change: (operation: RunningOperation) => RunningOperation): void {
  useJobsStore.setState((state) => {
    const current = state.running[id];
    return current ? { running: { ...state.running, [id]: change(current) } } : state;
  });
}

/**
 * Registers an operation for `item` and joins it to the batch. Returns
 * false (and changes nothing) when the file already has one.
 */
function begin(item: JobItem, progress: OperationProgress): boolean {
  if (useJobsStore.getState().running[item.id]) return false;
  useJobsStore.setState((state) => ({
    running: {
      ...state.running,
      [item.id]: { name: item.name, job: null, cancelRequested: false, progress },
    },
    batch: { size: (state.batch?.size ?? 0) + 1, ended: state.batch?.ended ?? [] },
  }));
  return true;
}

/**
 * Ends the operation of `item` with `outcome`; the file rests at `status`.
 * When it was the last one, the batch ends.
 */
function end(item: JobItem, outcome: Outcome, status: QueueStatus): void {
  queueStatusAt.delete(item.id);
  useQueueStore.getState().setStatus(item.id, status, outcome.output ?? undefined);
  useJobsStore.setState((state) => {
    const running = { ...state.running };
    delete running[item.id];
    const batch: Batch = {
      size: state.batch?.size ?? 1,
      ended: [...(state.batch?.ended ?? []), outcome],
    };
    const { output } = outcome;
    const recent =
      output === null
        ? state.recent
        : [
            { path: output, text: outcome.text },
            ...state.recent.filter((entry) => entry.path !== output),
          ].slice(0, RECENT_EXPORTS);
    return { running, batch, recent };
  });
  if (Object.keys(useJobsStore.getState().running).length === 0) endBatch();
}

/** Ends the batch: the queue leaves its pause, then the batch's notification and folder. */
function endBatch(): void {
  const { batch, paused } = useJobsStore.getState();
  useJobsStore.setState({ batch: null, paused: false });
  if (paused) {
    resumeQueue().catch((reason: unknown) => logWarning("resuming the queue failed", reason));
  }
  if (batch) batchEnded(batch);
}

/** Ends an operation the user canceled. */
function endCanceled(item: JobItem, operation: Operation): void {
  end(
    item,
    { name: item.name, operation, result: "canceled", text: "", review: false, output: null },
    { kind: "canceled" },
  );
}

/** Ends an operation whose job failed, with a toast that leads to the details. */
function fail(item: JobItem, operation: Operation, message: string, detail: string | null): void {
  const title = `${item.name} could not be ${verbOf(operation)}`;
  end(
    item,
    { name: item.name, operation, result: "failed", text: message, review: false, output: null },
    { kind: "failed", reason: message, detail },
  );
  toast.error(title, {
    description: message,
    action: { label: "Details", onClick: () => showFailureDetails({ title, message, detail }) },
  });
}

/** Ends an operation whose last job finished, with a toast. */
function succeed(item: JobItem, spec: OperationSpec, summary: JobSummary): void {
  const status = spec.finished(summary);
  const { text, review } = resultOf(status);
  const { output } = summary;
  end(
    item,
    { name: item.name, operation: spec.operation, result: "done", text, review, output },
    status,
  );
  spec.after();
  const verb = verbOf(spec.operation);
  const message = [
    output ? `${item.name} ${verb} as ${baseName(output)}` : `${item.name} ${verb}`,
    text,
  ]
    .filter((part) => part !== "")
    .join(" · ");
  const options = output
    ? { action: { label: "Show in folder", onClick: () => void showInFolder(output) } }
    : {};
  if (review) toast.warning(message, options);
  else toast.success(message, options);
}

/**
 * How often a running file's queue status follows its progress, in
 * milliseconds. The processing card shows every event; the queue row (and
 * everything that watches the queue) needs far fewer updates.
 */
const QUEUE_STATUS_INTERVAL_MS = 250;

/** When each running file's queue status last followed a progress event. */
const queueStatusAt = new Map<string, number>();

/**
 * Whether a progress event should also update the queue status of `id`.
 * Decided when an event arrives, without a timer, so a throttled webview
 * cannot delay it.
 */
function queueStatusDue(id: string): boolean {
  const now = performance.now();
  const last = queueStatusAt.get(id);
  if (last !== undefined && now - last < QUEUE_STATUS_INTERVAL_MS) return false;
  queueStatusAt.set(id, now);
  return true;
}

/**
 * Routes the events of the job at `step` of an operation: progress into
 * the card and the queue, `Finished` on to the next job or to the end.
 * Late events of an earlier job are ignored.
 */
function listen(item: JobItem, spec: OperationSpec, step: number): JobListener {
  return (event: JobEvent) => {
    const operation = useJobsStore.getState().running[item.id];
    if (!operation || operation.progress.step > step) return;
    switch (event.type) {
      case "Finished":
        if (step + 1 >= spec.steps.length) succeed(item, spec, event.summary);
        else if (operation.cancelRequested) endCanceled(item, spec.operation);
        else void startJob(item, spec, step + 1);
        break;
      case "Failed":
        fail(item, spec.operation, event.error.message, event.error.detail);
        break;
      case "Canceled":
        endCanceled(item, spec.operation);
        break;
      default: {
        const progress = applyJobEvent(operation.progress, event);
        patchRunning(item.id, (current) => ({ ...current, progress }));
        if (event.type !== "Progress" || queueStatusDue(item.id)) {
          useQueueStore.getState().setStatus(item.id, runningStatus(progress));
        }
      }
    }
  };
}

/**
 * Records the backend's id of the job at `step`; a cancel the user asked
 * for while the id was unknown is sent now.
 */
function jobKnown(id: string, step: number, job: JobId): void {
  const operation = useJobsStore.getState().running[id];
  if (!operation || operation.progress.step > step) return;
  patchRunning(id, (current) => ({ ...current, job }));
  if (operation.cancelRequested) void sendCancel(job);
}

/** Starts the job at `step` of an operation. */
async function startJob(item: JobItem, spec: OperationSpec, step: number): Promise<void> {
  const start = spec.steps[step]?.start;
  if (!start) return;
  patchRunning(item.id, (current) => ({
    ...current,
    job: null,
    progress: startStep(current.progress, step),
  }));
  try {
    jobKnown(item.id, step, await start(listen(item, spec, step)));
  } catch (reason) {
    const error = toAppError(reason);
    fail(item, spec.operation, error.message, error.detail);
  }
}

/** Starts an operation for `item`, unless one is already running for it. */
async function runOperation(item: JobItem, spec: OperationSpec): Promise<void> {
  const plan = spec.steps.map((step) => step.kind);
  const progress = startStep(waitingOperation(spec.operation, plan), 0);
  if (!begin(item, progress)) return;
  useQueueStore.getState().setStatus(item.id, runningStatus(progress));
  await startJob(item, spec, 0);
}

/** Reads a project, its tracks, its report and its output preview again after a job. */
function reloadProject(queryClient: QueryClient, path: string, projectId: string): void {
  void Promise.all([
    queryClient.invalidateQueries({ queryKey: ["project", path] }),
    queryClient.invalidateQueries({ queryKey: ["tracks", projectId] }),
    queryClient.invalidateQueries({ queryKey: ["verify-report", projectId] }),
    queryClient.invalidateQueries({ queryKey: ["output-preview", projectId] }),
  ]);
}

/**
 * Analyses (or re-analyses) a queued file with `settings`. When the job
 * finishes, the project, its tracks and its report are read again.
 */
export function analyzeItem(
  item: JobItem,
  projectId: string,
  settings: JobSettings,
  queryClient: QueryClient,
): Promise<void> {
  return runOperation(item, {
    operation: "analyze",
    steps: [
      {
        kind: "Analyze",
        start: (onEvent) => startAnalysis(analysisRequest(projectId, settings), onEvent),
      },
    ],
    finished: () => ({ kind: "ready", boxes: null }),
    after: () => reloadProject(queryClient, item.path, projectId),
  });
}

/** The resting status of an export, from its verification result (plan 7.2). */
export function exportedStatus(summary: JobSummary): QueueStatus {
  switch (summary.verify) {
    case null:
      return { kind: "done", verified: false };
    case "Clean":
      return { kind: "done", verified: true };
    default:
      // Not clean: the output is named `_UNVERIFIED` until the report is accepted.
      return { kind: "unverified" };
  }
}

/** A project as an export needs it. */
export interface ExportTarget {
  readonly id: string;
  /** Whether edits changed ranges since the last verification. */
  readonly unverified: boolean;
}

/**
 * Exports a queued file with `settings`. When edits changed ranges since
 * the last verification and verification is on, those ranges are
 * verified first (plan 7), so an export never ships unchecked edits.
 */
export function exportItem(
  item: JobItem,
  project: ExportTarget,
  settings: JobSettings,
  queryClient: QueryClient,
): Promise<void> {
  const render: StepSpec = {
    kind: "Render",
    start: (onEvent) => startRender(renderRequest(project.id, settings), onEvent),
  };
  const reverify: StepSpec = {
    kind: "ReverifyRanges",
    start: (onEvent) => startReverify(project.id, onEvent),
  };
  return runOperation(item, {
    operation: "export",
    steps: project.unverified && settings.verify.enabled ? [reverify, render] : [render],
    finished: exportedStatus,
    after: () => reloadProject(queryClient, item.path, project.id),
  });
}

/** One file of "Export N files". */
export interface RunEntry {
  readonly item: JobItem;
  readonly projectId: string;
  readonly settings: JobSettings;
}

/**
 * Hands several files to the backend's run queue ("Export N files").
 * Files that already have an operation are skipped. Every file waits
 * ("Waiting to export") until its job starts; events of jobs whose id
 * the command has not answered yet are kept and delivered once it has.
 */
export async function exportItems(
  entries: readonly RunEntry[],
  queryClient: QueryClient,
): Promise<void> {
  const accepted = entries.filter((entry) =>
    begin(entry.item, waitingOperation("export", ["Run"])),
  );
  if (accepted.length === 0) return;
  for (const { item } of accepted) {
    useQueueStore
      .getState()
      .setStatus(item.id, { kind: "waiting", prior: restingStatus(item.status) });
  }

  const listeners = new Map<JobId, JobListener>();
  const early = new Map<JobId, JobEvent[]>();
  const onEvent: JobListener = (event) => {
    const listener = listeners.get(event.job);
    if (listener) listener(event);
    else early.set(event.job, [...(early.get(event.job) ?? []), event]);
  };

  let jobs: JobId[];
  try {
    jobs = await startRun(
      accepted.map(({ projectId, settings }) => runFile(projectId, settings)),
      onEvent,
    );
  } catch (reason) {
    const error = toAppError(reason);
    for (const { item } of accepted) fail(item, "export", error.message, error.detail);
    return;
  }

  accepted.forEach(({ item, projectId }, index) => {
    const job = jobs[index];
    if (job === undefined) {
      fail(item, "export", "The export queue did not take this file.", null);
      return;
    }
    const listener = listen(
      item,
      {
        operation: "export",
        steps: [{ kind: "Run", start: null }],
        finished: exportedStatus,
        after: () => reloadProject(queryClient, item.path, projectId),
      },
      0,
    );
    listeners.set(job, listener);
    jobKnown(item.id, 0, job);
    for (const event of early.get(job) ?? []) listener(event);
    early.delete(job);
  });
}

/** Sends `cancel_job`, reporting a failure. */
async function sendCancel(job: JobId): Promise<void> {
  try {
    await cancelJob(job);
  } catch (reason) {
    logWarning("canceling a job failed", reason);
    toast.error("The job could not be canceled", { description: toAppError(reason).message });
  }
}

/**
 * Cancels the operation of a queued file, running or waiting, at any
 * stage. Before the backend has answered the job id, the cancel is
 * remembered and sent with the id.
 */
export async function cancelItem(id: string): Promise<void> {
  const operation = useJobsStore.getState().running[id];
  if (!operation) return;
  patchRunning(id, (current) => ({ ...current, cancelRequested: true }));
  if (operation.job) await sendCancel(operation.job);
}

/** Pauses or resumes every job (the processing card's Pause button). */
export async function setPaused(paused: boolean): Promise<void> {
  try {
    await (paused ? pauseQueue() : resumeQueue());
    useJobsStore.setState({ paused });
  } catch (reason) {
    logWarning(paused ? "pausing the queue failed" : "resuming the queue failed", reason);
    toast.error(paused ? "The jobs could not be paused" : "The jobs could not be resumed", {
      description: toAppError(reason).message,
    });
  }
}
