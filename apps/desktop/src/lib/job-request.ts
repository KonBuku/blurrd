/**
 * Job requests from job settings: every value the inspector shows goes
 * into the request unchanged; the regions are normalised (inspector
 * order, no duplicates) so equal settings give equal requests.
 */
import type { AnalysisRequest, RenderRequest, RunFile } from "@/ipc/jobs";

import { orderedRegions, type JobSettings } from "./job-settings";

/** The `start_analysis` request of a project with `settings`. */
export function analysisRequest(projectId: string, settings: JobSettings): AnalysisRequest {
  const { regions, verify } = settings;
  return {
    projectId,
    regions: { ...regions, enabled: orderedRegions(regions.enabled) },
    verify,
  };
}

/** The `start_render` request of a project with `settings`. */
export function renderRequest(projectId: string, settings: JobSettings): RenderRequest {
  return {
    projectId,
    settings: settings.export,
    blur: settings.blur,
    enabled: orderedRegions(settings.regions.enabled),
  };
}

/** One file of a `start_run` request: the analysis and render requests in one. */
export function runFile(projectId: string, settings: JobSettings): RunFile {
  const { settings: exportSettings, blur, enabled } = renderRequest(projectId, settings);
  return { ...analysisRequest(projectId, settings), settings: exportSettings, blur, enabled };
}
