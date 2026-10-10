/**
 * Jobs of the mock backend: a job sends `Started`, its stages and a few
 * `Progress` events through the channel it was started with, then
 * `Finished` with the summary its `finish` callback computes (or `Failed`,
 * or `Canceled` after `cancel_job`). Events go through Tauri's injected callback table, the
 * same path the real channel uses.
 */
import type { AppError } from "@/bindings/AppError";
import type { JobEvent } from "@/bindings/JobEvent";
import type { JobKind } from "@/bindings/JobKind";
import type { JobSummary } from "@/bindings/JobSummary";
import type { Stage } from "@/bindings/Stage";

/** The part of Tauri's injected internals a channel message goes through. */
interface TauriInternals {
  runCallback(id: number, data: unknown): void;
}

/** A channel as the mocked `invoke` receives it (the `Channel` instance). */
export interface MockChannel {
  readonly id: number;
}

/** Stages each kind of job reports, in order. */
const STAGES: Readonly<Record<JobKind, readonly Stage[]>> = {
  Analyze: ["Decode", "Detect", "Track", "Ocr", "Verify"],
  Render: ["BlurEncode", "Mux"],
  Run: ["Decode", "Detect", "Track", "Ocr", "Verify", "BlurEncode", "Mux"],
  Audit: ["Decode", "Audit"],
  ReverifyRanges: ["Decode", "Ocr", "Verify"],
};

/** Progress events per job. */
const PROGRESS_STEPS = 4;

/** Speed the mock jobs report, in frames per second. */
export const MOCK_JOB_FPS = 390;

function internals(): TauriInternals {
  // oxlint-disable-next-line no-underscore-dangle
  return (window as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__;
}

/** A running mock job. */
interface RunningJob {
  canceled: boolean;
}

/** Starts and cancels the jobs of one mock backend. */
export class MockJobs {
  /** How long a job runs, in milliseconds. */
  durationMs = 200;
  /** When set, the next job fails with this error (then it is cleared). */
  failNext: AppError | null = null;
  private counter = 0;
  private readonly running = new Map<string, RunningJob>();

  /**
   * Starts a job of `kind` over `totalFrames` frames that reports to
   * `channel`; `finish` runs when it completes and returns its summary.
   * Returns the job id at once.
   */
  start(
    kind: JobKind,
    totalFrames: number,
    channel: MockChannel,
    finish: () => JobSummary,
  ): string {
    this.counter += 1;
    const job = `job-${this.counter}`;
    const state: RunningJob = { canceled: false };
    this.running.set(job, state);
    let index = 0;
    const send = (message: JobEvent) => {
      internals().runCallback(channel.id, { index, message });
      index += 1;
    };
    const failure = this.failNext;
    this.failNext = null;
    void (async () => {
      // The command answers first, like the real backend.
      await pause(0);
      send({ type: "Started", job, kind, total_frames: totalFrames });
      const stages = STAGES[kind];
      for (let step = 1; step <= PROGRESS_STEPS; step += 1) {
        // oxlint-disable-next-line no-await-in-loop -- progress events are spaced out on purpose
        await pause(this.durationMs / PROGRESS_STEPS);
        if (state.canceled) break;
        const stage =
          stages[
            Math.min(stages.length - 1, Math.floor(((step - 1) * stages.length) / PROGRESS_STEPS))
          ];
        if (stage !== undefined) send({ type: "Stage", job, stage });
        const frame = Math.round((totalFrames * step) / PROGRESS_STEPS);
        const eta_ms = Math.round(((totalFrames - frame) / MOCK_JOB_FPS) * 1000);
        send({ type: "Progress", job, frame, total: totalFrames, fps: MOCK_JOB_FPS, eta_ms });
      }
      this.running.delete(job);
      if (state.canceled) send({ type: "Canceled", job });
      else if (failure) send({ type: "Failed", job, error: failure });
      else send({ type: "Finished", job, summary: finish() });
    })();
    return job;
  }

  /** `cancel_job`: stops a running job; unknown or finished jobs are ignored. */
  cancel(job: string): void {
    const state = this.running.get(job);
    if (state) state.canceled = true;
  }
}

function pause(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}
