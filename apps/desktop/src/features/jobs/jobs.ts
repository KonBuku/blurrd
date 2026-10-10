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
import type { JobKind } from "@/bindings/JobKind";
import { toAppError } from "@/ipc/errors";
import { startAnalysis } from "@/ipc/jobs";
import { analysisRequest } from "@/lib/job-request";
import type { JobSettings } from "@/lib/job-settings";
import type { QueueStatus } from "@/lib/queue-status";
import { useQueueStore, type QueueItem } from "@/store/queue";

/** A job of a queued file. */
export interface RunningJob {
  readonly kind: JobKind;
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

/** The queue status while an analysis is at `progress` (0–1). */
function analyzing(progress: number, fps: number | null): QueueStatus {
  return { kind: "analyzing", progress, fps };
}

/**
 * Analyses (or re-analyses) a queued file with `settings`. Resolves once
 * the job has started; its events then update the queue. When the job
 * finishes, the project, its tracks and its report are read again.
 */
export async function analyzeItem(
  item: Pick<QueueItem, "id" | "path" | "name" | "status">,
  projectId: string,
  settings: JobSettings,
  queryClient: QueryClient,
): Promise<void> {
  if (useJobsStore.getState().running[item.id]) return;
  const queue = useQueueStore.getState();
  setRunning(item.id, { kind: "Analyze" });
  queue.setStatus(item.id, analyzing(0, null));

  const onEvent = (event: JobEvent) => {
    switch (event.type) {
      case "Progress":
        queue.setStatus(
          item.id,
          analyzing(event.total > 0 ? event.frame / event.total : 0, event.fps),
        );
        break;
      case "Finished":
        setRunning(item.id, null);
        queue.setStatus(item.id, { kind: "ready", boxes: null });
        void Promise.all([
          queryClient.invalidateQueries({ queryKey: ["project", item.path] }),
          queryClient.invalidateQueries({ queryKey: ["tracks", projectId] }),
          queryClient.invalidateQueries({ queryKey: ["verify-report", projectId] }),
        ]);
        toast.success(`${item.name} analyzed`);
        break;
      case "Failed":
        setRunning(item.id, null);
        queue.setStatus(item.id, {
          kind: "failed",
          reason: event.error.message,
          detail: event.error.detail,
        });
        toast.error(`${item.name} could not be analyzed`, { description: event.error.message });
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
    await startAnalysis(analysisRequest(projectId, settings), onEvent);
  } catch (reason) {
    setRunning(item.id, null);
    queue.setStatus(item.id, item.status);
    toast.error(`${item.name} could not be analyzed`, { description: toAppError(reason).message });
  }
}
