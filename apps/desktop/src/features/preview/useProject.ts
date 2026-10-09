import { useQuery } from "@tanstack/react-query";
import { useMemo } from "react";

import type { AppError } from "@/bindings/AppError";
import type { Region } from "@/bindings/Region";
import { getPts, getTracks, openProject, type ProjectSummary, type TrackSet } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { colourModel, pixelParams, type ColourModel, type PixelParams } from "@/lib/blur-geometry";
import { FrameClock } from "@/lib/frames";
import { IntervalIndex } from "@/lib/interval-index";

import { useProjectEdits } from "@/features/editing/useProjectEdits";

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
 * per project for the session). The edits come from the edit store, which
 * loads and saves them (`useProjectEdits`), so the index follows every
 * edit, undo and redo at once.
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
  const overrides = useProjectEdits(path, id, tracks.data);
  // The clock only depends on the timestamps: a saved edit updates the
  // project's verify state, which must not restart the playback.
  const [fpsNum, fpsDen] = project.data?.media.fps ?? [0, 0];
  const clock = useMemo(
    () => (pts.data ? new FrameClock(pts.data.pts, pts.data.time_base, [fpsNum, fpsDen]) : null),
    [pts.data, fpsNum, fpsDen],
  );
  const data = useMemo((): LoadedProject | null => {
    if (!project.data || !tracks.data || !clock || !overrides) return null;
    const { media } = project.data;
    return {
      project: project.data,
      // The edits on screen, saved or about to be (the edit store owns them).
      tracks: { ...tracks.data, overrides },
      clock,
      index: new IntervalIndex(clock.count, tracks.data.tracks, overrides),
      params: pixelParams(project.data.blur, project.data.sy),
      colour: colourModel(media.color.matrix, media.color.range),
      enabled: new Set(project.data.enabled),
    };
  }, [project.data, tracks.data, clock, overrides]);
  const failure = project.error ?? tracks.error ?? pts.error;
  if (failure) return { status: "error", error: toAppError(failure) };
  return data ? { status: "ready", data } : { status: "loading" };
}
