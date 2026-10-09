import { useQuery } from "@tanstack/react-query";

import type { DensityLanes } from "@/bindings/DensityLanes";
import type { VerifyReport } from "@/bindings/VerifyReport";
import { getTimelineDensity, getVerifyReport } from "@/ipc/commands";

/** Density buckets requested per video; the lanes resample them to pixels. */
export const DENSITY_BUCKETS = 2048;

/** Region coverage per timeline bucket, cached per project revision. */
export function useDensity(projectId: string, revision: number, frameCount: number) {
  const buckets = Math.max(1, Math.min(frameCount, DENSITY_BUCKETS));
  return useQuery<DensityLanes>({
    queryKey: ["density", projectId, revision, buckets],
    queryFn: () => getTimelineDensity(projectId, buckets),
  });
}

/**
 * The last verification report of a project (`null` before the first
 * one); an empty id (project still loading) reads nothing.
 */
export function useVerifyReport(projectId: string) {
  return useQuery<VerifyReport | null>({
    queryKey: ["verify-report", projectId],
    queryFn: () => getVerifyReport(projectId),
    enabled: projectId !== "",
  });
}
