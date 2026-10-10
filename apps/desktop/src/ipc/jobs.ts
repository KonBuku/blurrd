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
import type { VerifySettings } from "@/bindings/VerifySettings";
import type { DetectedRegion, DetectorOptions, RegionChoice } from "@/lib/job-settings";

/** Arguments of `start_analysis` without the channel. */
export interface AnalysisRequest {
  readonly projectId: string;
  readonly regions: RegionChoice;
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

/** Cancels a running job; it then reports `Canceled`. */
export function cancelJob(job: JobId): Promise<void> {
  return invoke<void>("cancel_job", { job });
}
