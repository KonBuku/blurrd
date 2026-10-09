import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import type { AppError } from "@/bindings/AppError";
import type { Region } from "@/bindings/Region";
import { getPts, getTracks, openProject, type ProjectSummary, type TrackSet } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { colourModel, pixelParams, type ColourModel, type PixelParams } from "@/lib/blur-geometry";
import { FrameClock } from "@/lib/frames";
import { IntervalIndex } from "@/lib/interval-index";

/** Everything the preview needs about the shown file, once loaded. */
export interface LoadedProject {
  readonly project: ProjectSummary;
  readonly tracks: TrackSet;
  readonly clock: FrameClock;
  readonly index: IntervalIndex;
  /** Blur sizes in source pixels; `null` for parameters the engine rejects. */
  readonly params: PixelParams | null;
  readonly colour: ColourModel;
  readonly enabled: ReadonlySet<Region>;
}

/** The preview project of `path`: loading, failed or loaded. */
export type ProjectState =
  | { readonly status: "loading" }
  | { readonly status: "error"; readonly error: AppError }
  | { readonly status: "ready"; readonly data: LoadedProject };

/**
 * Opens the project of `path` and reads its tracks and timestamps (cached
 * per project for the session; a new revision is a new query).
 */
export function useProject(path: string): ProjectState {
  const project = useQuery({ queryKey: ["project", path], queryFn: () => openProject(path) });
  const id = project.data?.id;
  const tracks = useQuery({
    queryKey: ["tracks", id],
    queryFn: () => getTracks(id ?? ""),
    enabled: id !== undefined,
  });
  const pts = useQuery({
    queryKey: ["pts", id],
    queryFn: () => getPts(id ?? ""),
    enabled: id !== undefined,
  });
  const data = useMemo((): LoadedProject | null => {
    if (!project.data || !tracks.data || !pts.data) return null;
    const { media } = project.data;
    return {
      project: project.data,
      tracks: tracks.data,
      clock: new FrameClock(pts.data.pts, pts.data.time_base, media.fps),
      index: new IntervalIndex(pts.data.pts.length, tracks.data.tracks, tracks.data.overrides),
      params: pixelParams(project.data.blur, project.data.sy),
      colour: colourModel(media.color.matrix, media.color.range),
      enabled: new Set(project.data.enabled),
    };
  }, [project.data, tracks.data, pts.data]);
  const failure = project.error ?? tracks.error ?? pts.error;
  if (failure) return { status: "error", error: toAppError(failure) };
  return data ? { status: "ready", data } : { status: "loading" };
}
