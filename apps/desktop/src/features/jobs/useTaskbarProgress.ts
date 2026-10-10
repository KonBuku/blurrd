import { useEffect } from "react";

import { setTaskbarProgress } from "@/ipc/system";
import { TASKBAR_IDLE, taskbarState, type TaskbarState } from "@/lib/batch";
import { logWarning } from "@/lib/log";

import { useJobsStore, type JobsState } from "./jobs";

/** The taskbar state of the jobs store. */
export function jobsTaskbarState(state: JobsState): TaskbarState {
  const running = Object.fromEntries(
    Object.entries(state.running).map(([id, operation]) => [id, operation.progress]),
  );
  return taskbarState(state.batch, running, state.paused);
}

/**
 * Mirrors the batch's overall progress on the window's taskbar button
 * (design 8.3). The button is only told about changes of the status or
 * the whole percentage, so a batch costs at most about a hundred calls
 * however often jobs report. Store subscriptions run on job events, not
 * on animation frames, so the button moves on while the window is
 * minimised.
 */
export function useTaskbarProgress(): void {
  useEffect(() => {
    let shown: TaskbarState = TASKBAR_IDLE;
    const show = (state: JobsState) => {
      const next = jobsTaskbarState(state);
      if (next.status === shown.status && next.progress === shown.progress) return;
      shown = next;
      setTaskbarProgress(next).catch((reason: unknown) =>
        logWarning("updating the taskbar progress failed", reason),
      );
    };
    show(useJobsStore.getState());
    return useJobsStore.subscribe(show);
  }, []);
}
