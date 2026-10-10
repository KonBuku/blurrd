/**
 * Job requests from job settings: every value the inspector shows goes
 * into the request unchanged; lists are normalised (regions in inspector
 * order, words trimmed and distinct) so equal settings give equal requests.
 */
import type { AnalysisRequest } from "@/ipc/jobs";

import { orderedRegions, parseWordList, type JobSettings } from "./job-settings";

/** The `start_analysis` request of a project with `settings`. */
export function analysisRequest(projectId: string, settings: JobSettings): AnalysisRequest {
  const { regions, detector, verify } = settings;
  return {
    projectId,
    regions: {
      ...regions,
      enabled: orderedRegions(regions.enabled),
      keep_names: {
        ...regions.keep_names,
        names: parseWordList(regions.keep_names.names.join("\n")),
      },
    },
    detector,
    verify: { ...verify, ignore_words: parseWordList(verify.ignore_words.join("\n")) },
  };
}
