import { useQuery } from "@tanstack/react-query";

import type { ProjectSummary } from "@/bindings/ProjectSummary";
import type { TrackSet } from "@/bindings/TrackSet";
import { useEqualSelector } from "@/hooks/useEqualSelector";

import { getTracks, openProject } from "@/ipc/commands";
import type { JobSettings, SettingsSection } from "@/lib/job-settings";
import { settingsOf, targetsOf, useJobSettingsStore } from "@/store/job-settings";
import { shownItemId, useQueueStore, type QueueItem } from "@/store/queue";

/** What the inspector edits and where a change goes. */
export interface InspectorTarget {
  /** The file whose values are shown, or `null` for the defaults of new files. */
  readonly lead: QueueItem | null;
  /** The shown values. */
  readonly settings: JobSettings;
  /** Number of files a change applies to. */
  readonly fileCount: number;
  /** Whether a change also sets the defaults for files added later. */
  readonly setsDefaults: boolean;
  /** Changes one section of every target (see `updateSection`). */
  readonly update: <K extends SettingsSection>(
    key: K,
    change: (section: JobSettings[K]) => JobSettings[K],
  ) => void;
}

/**
 * Whether two versions of the shown item look the same to the inspector:
 * same file, media, output and kind of status. Job progress (20 updates a
 * second) changes neither, so it does not render the inspector again.
 */
function sameLead(a: QueueItem | null, b: QueueItem | null): boolean {
  if (a === b) return true;
  if (a === null || b === null) return false;
  return (
    a.id === b.id &&
    a.path === b.path &&
    a.name === b.name &&
    a.media === b.media &&
    a.output === b.output &&
    a.status.kind === b.status.kind
  );
}

/**
 * The inspector's target: the values of the shown file (or the defaults
 * when no file is selected) and an `update` that writes the selected
 * files, or every file when "Apply to" is "All".
 */
export function useInspectorTarget(): InspectorTarget {
  const scope = useJobSettingsStore((state) => state.scope);
  const lead = useQueueStore(
    useEqualSelector((queue) => {
      const id = shownItemId(queue);
      return queue.items.find((item) => item.id === id) ?? null;
    }, sameLead),
  );
  const fileCount = useQueueStore((queue) => targetsOf(queue, scope).ids.length);
  const setsDefaults = useQueueStore((queue) => targetsOf(queue, scope).defaults);
  const settings = useJobSettingsStore((state) => settingsOf(state, lead?.id ?? null));
  const updateSection = useJobSettingsStore((state) => state.updateSection);
  return {
    lead,
    settings,
    fileCount,
    setsDefaults,
    update: (key, change) => updateSection(targetsOf(useQueueStore.getState(), scope), key, change),
  };
}

/**
 * The opened project and tracks of a queued file, shared with the preview
 * through the query cache; nothing while the file is not a probed video.
 */
export function useLeadProject(lead: QueueItem | null): {
  readonly project: ProjectSummary | undefined;
  readonly tracks: TrackSet | undefined;
} {
  const path = lead?.media ? lead.path : null;
  const project = useQuery({
    queryKey: ["project", path],
    queryFn: () => openProject(path ?? ""),
    enabled: path !== null,
  });
  const id = project.data?.id;
  const tracks = useQuery({
    queryKey: ["tracks", id],
    queryFn: () => getTracks(id ?? ""),
    enabled: id !== undefined,
  });
  return { project: project.data, tracks: tracks.data };
}
