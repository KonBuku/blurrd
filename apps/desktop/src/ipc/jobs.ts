/**
 * The job commands (plan 9.1). A job reports its progress through a
 * `Channel<JobEvent>` passed with the command; the command itself answers
 * with the job id once the job was queued. The request shapes are built
 * from the inspector's job settings in `lib/job-request.ts`.
 */
import { Channel, invoke } from "@tauri-apps/api/core";

import type { JobEvent } from "@/bindings/JobEvent";
import type { JobId } from "@/bindings/JobId";
import type { VerifySettings } from "@/bindings/VerifySettings";
import type { DetectorOptions, RegionChoice } from "@/lib/job-settings";

/** Arguments of `start_analysis` without the channel. */
export interface AnalysisRequest {
  readonly projectId: string;
  readonly regions: RegionChoice;
  readonly detector: DetectorOptions;
  readonly verify: VerifySettings;
}

/** Called with every event of a job, in order. */
export type JobListener = (event: JobEvent) => void;

/** Analyses (or re-analyses) a project: detection, tracking and verification. */
export function startAnalysis(request: AnalysisRequest, onEvent: JobListener): Promise<JobId> {
  return invoke<JobId>("start_analysis", { ...request, onEvent: new Channel(onEvent) });
}
