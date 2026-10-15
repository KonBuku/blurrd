import { useQueryClient } from "@tanstack/react-query";
import { useEffect, useMemo } from "react";
import { toast } from "sonner";

import type { Override } from "@/bindings/Override";
import type { ProjectSummary } from "@/bindings/ProjectSummary";
import type { TrackSet } from "@/bindings/TrackSet";
import { getTracks, openProject, setOverrides } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { overrideBindings } from "@/lib/interval-index";
import { useEditsStore } from "@/store/edits";

import { OverrideSync } from "./override-sync";

/** The sync of the shown project, for the controls that save or retry. */
let current: OverrideSync | null = null;

/** Saves waiting edits of the shown project now (the Retry action). */
export function flushEdits(): Promise<void> {
  return current?.flush() ?? Promise.resolve();
}

/**
 * Connects the edit store to the project at `path` (id `projectId`, its
 * stored `server` tracks): loads the stored edits, saves every change
 * (`OverrideSync`), keeps the cached tracks and the project's unverified
 * ranges up to date after a save, reloads after a stale revision and
 * takes over the stored edits when the project changed elsewhere (a new
 * revision while nothing is being saved). Returns the edits to show, or
 * `null` until the store holds this project.
 */
export function useProjectEdits(
  path: string,
  projectId: string | undefined,
  server: TrackSet | undefined,
): readonly Override[] | null {
  const queryClient = useQueryClient();
  const sync = useMemo(() => {
    if (projectId === undefined) return null;
    const created: OverrideSync = new OverrideSync(projectId, {
      save: setOverrides,
      onSaved: (saved, overrides) => {
        queryClient.setQueryData<TrackSet>(
          ["tracks", projectId],
          (old) =>
            old && {
              ...old,
              revision: saved.revision,
              overrides: [...overrides],
              bindings: overrideBindings(old.tracks, overrides),
            },
        );
        queryClient.setQueryData<ProjectSummary>(
          ["project", path],
          (old) =>
            old && {
              ...old,
              verify: { ...old.verify, unverified_ranges: [...saved.unverified_ranges] },
            },
        );
      },
      onStale: async () => {
        try {
          const [tracks] = await Promise.all([
            queryClient.fetchQuery({
              queryKey: ["tracks", projectId],
              queryFn: () => getTracks(projectId),
              staleTime: 0,
            }),
            queryClient.fetchQuery({
              queryKey: ["project", path],
              queryFn: () => openProject(path),
              staleTime: 0,
            }),
          ]);
          created.adopt(tracks.revision, tracks.overrides);
          toast.warning("The edits were changed elsewhere", {
            description:
              "The latest saved edits of this recording are shown; your last change was not saved.",
          });
        } catch (reason) {
          useEditsStore.getState().setStatus("failed");
          toast.error("The edits could not be reloaded", {
            description: toAppError(reason).message,
          });
        }
      },
      onError: (error) => {
        toast.error("The edits could not be saved", {
          description: error.message,
          action: { label: "Retry", onClick: () => void created.flush() },
        });
      },
    });
    return created;
  }, [projectId, path, queryClient]);

  useEffect(() => {
    if (!sync) return undefined;
    sync.start();
    current = sync;
    return () => {
      sync.dispose();
      if (current === sync) current = null;
    };
  }, [sync]);

  // Load the stored edits, and take them over when they changed elsewhere.
  useEffect(() => {
    if (!sync || !server || projectId === undefined) return;
    const store = useEditsStore.getState();
    const changedElsewhere = server.revision !== store.baseRev && !sync.busy();
    if (!sync.loaded() || store.projectId !== projectId || changedElsewhere) {
      sync.adopt(server.revision, server.overrides);
    }
  }, [sync, server, projectId]);

  return useEditsStore((state) =>
    sync?.loaded() && state.projectId === projectId ? state.overrides : null,
  );
}
