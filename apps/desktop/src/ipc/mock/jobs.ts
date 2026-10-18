/**
 * Jobs of the mock backend: a job sends `Started`, each of its stages
 * with `Progress` events, then `Finished` with the summary its `finish`
 * callback computes (or `Failed`, or `Canceled` after `cancel_job`).
 * Run-queue jobs (`start_run`) wait for the job before them, like the
 * backend's queue with one job at a time; `pause_queue` holds every job
 * at its next progress step. Events go through Tauri's injected callback
 * table, the same path the real channel uses.
 */
import type { AppError } from "@/bindings/AppError";
import type { JobEvent } from "@/bindings/JobEvent";
import type { JobKind } from "@/bindings/JobKind";
import type { JobSummary } from "@/bindings/JobSummary";
import { JOB_STAGES } from "@/lib/job-stages";

/** The part of Tauri's injected internals a channel message goes through. */
interface TauriInternals {
  runCallback(id: number, data: unknown): void;
}

/** A channel as the mocked `invoke` receives it (the `Channel` instance). */
export interface MockChannel {
  readonly id: number;
}

/** Progress events per stage, at least. */
const STEPS_PER_STAGE = 2;

/** Rate of progress events of long jobs: the backend limit (plan M22, at most 20 Hz). */
export const MOCK_PROGRESS_HZ = 20;

/** Speed the mock jobs report, in frames per second. */
export const MOCK_JOB_FPS = 390;

/** How often a paused job looks whether it may go on, in milliseconds. */
const PAUSE_POLL_MS = 20;

function internals(): TauriInternals {
  // oxlint-disable-next-line no-underscore-dangle
  return (window as unknown as { __TAURI_INTERNALS__: TauriInternals }).__TAURI_INTERNALS__;
}

/** A job the mock knows: running, or waiting in the run queue. */
interface MockJob {
  canceled: boolean;
  /** Whether the job has started (sent `Started`). */
  started: boolean;
  readonly send: (message: JobEvent) => void;
}

/** One file of a run-queue request. */
export interface QueuedJob {
  /** What the job does (a `Run`, or a `Render` when the analysis is current). */
  readonly kind: JobKind;
  readonly totalFrames: number;
  readonly finish: () => JobSummary;
}

/** Sends the messages of one channel with their running index. */
function channelSender(channel: MockChannel): (message: JobEvent) => void {
  let index = 0;
  return (message) => {
    internals().runCallback(channel.id, { index, message });
    index += 1;
  };
}

/** Starts, queues, pauses and cancels the jobs of one mock backend. */
export class MockJobs {
  /** How long a job runs, in milliseconds. */
  durationMs = 200;
  /** When set, the next job fails with this error (then it is cleared). */
  failNext: AppError | null = null;
  /** Whether `pause_queue` holds the jobs. */
  paused = false;
  private counter = 0;
  /** Jobs that have not ended, by id. */
  private readonly jobs = new Map<string, MockJob>();
  /** The end of the last run-queue job; the next one starts after it. */
  private queueTail: Promise<void> = Promise.resolve();
  /**
   * The backend's clock: the page's `setTimeout` as it was when the mock
   * was installed. A test that throttles the page's timers (as a browser
   * does for a hidden window) leaves the jobs running at their own pace,
   * like the real engine, which does not run in the webview.
   */
  private readonly timer = globalThis.setTimeout.bind(globalThis);

  /** Whether no job runs or waits. */
  isIdle(): boolean {
    return this.jobs.size === 0;
  }

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
    const job = this.register(channelSender(channel));
    void this.run(job, { kind, totalFrames, finish });
    return job;
  }

  /**
   * `start_run`: queues one job per entry behind the jobs already queued;
   * all report to `channel`. Returns the job ids at once, in order.
   */
  enqueue(entries: readonly QueuedJob[], channel: MockChannel): string[] {
    const send = channelSender(channel);
    return entries.map((entry) => {
      const job = this.register(send);
      this.queueTail = this.queueTail.then(() => this.run(job, entry));
      return job;
    });
  }

  /**
   * `cancel_job`: stops a running job at its next step; a waiting job
   * reports `Canceled` at once and never starts. Unknown or ended jobs
   * are ignored.
   */
  cancel(job: string): void {
    const state = this.jobs.get(job);
    if (!state || state.canceled) return;
    state.canceled = true;
    if (!state.started) this.end(job, state, { type: "Canceled", job });
  }

  private register(send: (message: JobEvent) => void): string {
    this.counter += 1;
    const job = `job-${this.counter}`;
    this.jobs.set(job, { canceled: false, started: false, send });
    return job;
  }

  private end(job: string, state: MockJob, message: JobEvent): void {
    this.jobs.delete(job);
    state.send(message);
  }

  private pause(ms: number): Promise<void> {
    return new Promise((resolve) => {
      this.timer(resolve, ms);
    });
  }

  /** Runs one job to its end. */
  private async run(job: string, { kind, totalFrames, finish }: QueuedJob): Promise<void> {
    const state = this.jobs.get(job);
    if (!state) return;
    state.started = true;
    const failure = this.failNext;
    this.failNext = null;
    // The command answers first, like the real backend.
    await this.pause(0);
    state.send({ type: "Started", job, kind, total_frames: totalFrames, passes: 1 });
    const stages = JOB_STAGES[kind];
    const steps = Math.max(
      stages.length * STEPS_PER_STAGE,
      Math.round((this.durationMs / 1000) * MOCK_PROGRESS_HZ),
    );
    let shown = -1;
    for (let step = 1; step <= steps; step += 1) {
      // Instant jobs (tests) send their steps back to back.
      // oxlint-disable-next-line no-await-in-loop -- progress events are spaced out on purpose
      if (this.durationMs > 0) await this.pause(this.durationMs / steps);
      // oxlint-disable-next-line no-await-in-loop -- a paused job waits in place
      while (this.paused && !state.canceled) await this.pause(PAUSE_POLL_MS);
      if (state.canceled) break;
      const at = Math.min(stages.length - 1, Math.floor(((step - 1) * stages.length) / steps));
      const stage = stages[at];
      if (stage !== undefined && at !== shown) {
        shown = at;
        state.send({ type: "Stage", job, stage, pass: 0 });
      }
      const frame = Math.round((totalFrames * step) / steps);
      const eta_ms = Math.round(((totalFrames - frame) / MOCK_JOB_FPS) * 1000);
      state.send({ type: "Progress", job, frame, total: totalFrames, fps: MOCK_JOB_FPS, eta_ms });
    }
    if (state.canceled) this.end(job, state, { type: "Canceled", job });
    else if (failure) this.end(job, state, { type: "Failed", job, error: failure });
    else this.end(job, state, { type: "Finished", job, summary: finish() });
  }
}
