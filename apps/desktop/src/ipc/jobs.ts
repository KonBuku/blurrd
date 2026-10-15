/**
 * The job commands (plan 9.1). A job reports its progress through a
 * `Channel<JobEvent>` passed with the command; the command itself answers
 * with the job id once the job was queued. The request shapes are built
 * from the inspector's job settings in `lib/job-request.ts`.
 */
import { Channel, invoke } from "@tauri-apps/api/core";

import type { BlurParams } from "@/bindings/BlurParams";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { JobEvent } from "@/bindings/JobEvent";
import type { JobId } from "@/bindings/JobId";
import type { RegionSettings } from "@/bindings/RegionSettings";
import type { VerifySettings } from "@/bindings/VerifySettings";
import type { DetectedRegion, DetectorOptions } from "@/lib/job-settings";

/** Arguments of `start_analysis` without the channel. */
export interface AnalysisRequest {
  readonly projectId: string;
  readonly regions: RegionSettings;
  readonly detector: DetectorOptions;
  readonly verify: VerifySettings;
}

/**
 * Arguments of `start_render` without the channel. Besides the export
 * settings and the blur (plan 9.1) the request names the regions that
 * are blurred, so a region switched off after the analysis is left out
 * of the export like it is left out of the preview.
 */
export interface RenderRequest {
  readonly projectId: string;
  readonly settings: ExportSettings;
  readonly blur: BlurParams;
  readonly enabled: readonly DetectedRegion[];
}

/**
 * One file of a `start_run` request: everything the analysis and the
 * render of that file need, because settings are per file.
 */
export interface RunFile extends AnalysisRequest {
  readonly settings: ExportSettings;
  readonly blur: BlurParams;
  readonly enabled: readonly DetectedRegion[];
}

/** Called with every event of a job, in order. */
export type JobListener = (event: JobEvent) => void;

/** Analyses (or re-analyses) a project: detection, tracking and verification. */
export function startAnalysis(request: AnalysisRequest, onEvent: JobListener): Promise<JobId> {
  return invoke<JobId>("start_analysis", { ...request, onEvent: new Channel(onEvent) });
}

/** Blurs and encodes a project into a new file. */
export function startRender(request: RenderRequest, onEvent: JobListener): Promise<JobId> {
  return invoke<JobId>("start_render", { ...request, onEvent: new Channel(onEvent) });
}

/**
 * Verifies the frame ranges that edits changed since the last
 * verification; runs before an export of an edited project (plan 7).
 */
export function startReverify(projectId: string, onEvent: JobListener): Promise<JobId> {
  return invoke<JobId>("start_reverify", { projectId, onEvent: new Channel(onEvent) });
}

/**
 * Queues the export of several files in the backend's run queue ("Export
 * N files"). The backend analyses a file in the same pass unless its
 * analysis is current, and runs the files one after another; their
 * events share the channel and carry the job ids, which the command
 * answers in the order of `files`.
 */
export function startRun(files: readonly RunFile[], onEvent: JobListener): Promise<JobId[]> {
  return invoke<JobId[]>("start_run", { files, onEvent: new Channel(onEvent) });
}

/**
 * Cancels a job, running or still waiting in the run queue; it then
 * reports `Canceled`.
 */
export function cancelJob(job: JobId): Promise<void> {
  return invoke<void>("cancel_job", { job });
}

/** Pauses every job at the next frame; queued jobs wait. */
export function pauseQueue(): Promise<void> {
  return invoke<void>("pause_queue");
}

/** Resumes the jobs paused by `pauseQueue`. */
export function resumeQueue(): Promise<void> {
  return invoke<void>("resume_queue");
}
