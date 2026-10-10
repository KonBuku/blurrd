/**
 * Runs jobs for queued files and mirrors their events in the queue status
 * (design 8.1 status model): progress while running, then the resting
 * status, a failure with its message, and a toast. The processing card,
 * taskbar progress and notifications build on this (M27).
 */
import type { QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { create } from "zustand";

import type { JobEvent } from "@/bindings/JobEvent";
import type { JobId } from "@/bindings/JobId";
import type { JobKind } from "@/bindings/JobKind";
import type { JobSummary } from "@/bindings/JobSummary";
import { toAppError } from "@/ipc/errors";
import { cancelJob, startAnalysis, startRender, type JobListener } from "@/ipc/jobs";
import { analysisRequest, renderRequest } from "@/lib/job-request";
import type { JobSettings } from "@/lib/job-settings";
import { logWarning } from "@/lib/log";
import { baseName } from "@/lib/media";
import type { QueueStatus } from "@/lib/queue-status";
import { useQueueStore, type QueueItem } from "@/store/queue";

/** A job of a queued file. */
export interface RunningJob {
  readonly kind: JobKind;
  /** The backend's id, once the start command has answered. */
  readonly job: JobId | null;
}

/** Jobs that are running, by queue item id (one per file at a time). */
export const useJobsStore = create<{ readonly running: Readonly<Record<string, RunningJob>> }>()(
  () => ({ running: {} }),
);

function setRunning(id: string, job: RunningJob | null): void {
  useJobsStore.setState((state) => {
    const running = { ...state.running };
    if (job) running[id] = job;
    else delete running[id];
    return { running };
  });
}

/** The file a job runs for. */
type JobItem = Pick<QueueItem, "id" | "path" | "name" | "status">;

/** What differs between the kinds of jobs. */
interface JobSpec {
  readonly kind: JobKind;
  /** "analyzed" / "exported", for the messages. */
  readonly verb: string;
  /** The queue status at `progress` (0–1). */
  readonly running: (progress: number, fps: number | null, etaMs: number | null) => QueueStatus;
  /** The resting status after a finished job. */
  readonly finished: (summary: JobSummary) => QueueStatus;
  readonly start: (onEvent: JobListener) => Promise<JobId>;
  /** Runs after `Finished` (reloading what the job changed). */
  readonly after: () => void;
}

/**
 * Starts a job for `item`, unless one is already running for it, and
 * routes its events to the queue. Resolves once the job has started.
 */
async function runJob(item: JobItem, spec: JobSpec): Promise<void> {
  if (useJobsStore.getState().running[item.id]) return;
  const queue = useQueueStore.getState();
  setRunning(item.id, { kind: spec.kind, job: null });
  queue.setStatus(item.id, spec.running(0, null, null));

  const onEvent = (event: JobEvent) => {
    switch (event.type) {
      case "Progress":
        queue.setStatus(
          item.id,
          spec.running(event.total > 0 ? event.frame / event.total : 0, event.fps, event.eta_ms),
        );
        break;
      case "Finished": {
        setRunning(item.id, null);
        const { output } = event.summary;
        queue.setStatus(item.id, spec.finished(event.summary), output ?? undefined);
        spec.after();
        toast.success(
          output ? `${item.name} ${spec.verb} as ${baseName(output)}` : `${item.name} ${spec.verb}`,
        );
        break;
      }
      case "Failed":
        setRunning(item.id, null);
        queue.setStatus(item.id, {
          kind: "failed",
          reason: event.error.message,
          detail: event.error.detail,
        });
        toast.error(`${item.name} could not be ${spec.verb}`, { description: event.error.message });
        break;
      case "Canceled":
        setRunning(item.id, null);
        queue.setStatus(item.id, { kind: "canceled" });
        break;
      default:
        // Stages, warnings and leaks are shown by the processing card (M27).
        break;
    }
  };

  try {
    const job = await spec.start(onEvent);
    const running = useJobsStore.getState().running[item.id];
    if (running) setRunning(item.id, { ...running, job });
  } catch (reason) {
    setRunning(item.id, null);
    queue.setStatus(item.id, item.status);
    toast.error(`${item.name} could not be ${spec.verb}`, {
      description: toAppError(reason).message,
    });
  }
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
  return runJob(item, {
    kind: "Analyze",
    verb: "analyzed",
    running: (progress, fps) => ({ kind: "analyzing", progress, fps }),
    finished: () => ({ kind: "ready", boxes: null }),
    start: (onEvent) => startAnalysis(analysisRequest(projectId, settings), onEvent),
    after: () => {
      void Promise.all([
        queryClient.invalidateQueries({ queryKey: ["project", item.path] }),
        queryClient.invalidateQueries({ queryKey: ["tracks", projectId] }),
        queryClient.invalidateQueries({ queryKey: ["verify-report", projectId] }),
      ]);
    },
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

/** Exports a queued file with `settings`. */
export function exportItem(
  item: JobItem,
  projectId: string,
  settings: JobSettings,
  queryClient: QueryClient,
): Promise<void> {
  return runJob(item, {
    kind: "Render",
    verb: "exported",
    running: (progress, fps, etaMs) => ({ kind: "exporting", progress, fps, etaMs }),
    finished: exportedStatus,
    start: (onEvent) => startRender(renderRequest(projectId, settings), onEvent),
    after: () => {
      void queryClient.invalidateQueries({ queryKey: ["output-preview", projectId] });
    },
  });
}

/** Cancels the job running for a queued file, if any. */
export async function cancelItem(id: string): Promise<void> {
  const job = useJobsStore.getState().running[id]?.job;
  if (!job) return;
  try {
    await cancelJob(job);
  } catch (reason) {
    logWarning("canceling a job failed", reason);
    toast.error("The job could not be canceled", { description: toAppError(reason).message });
  }
}
