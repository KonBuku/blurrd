import { useQuery } from "@tanstack/react-query";

import { getTracks, openProject, type ProjectSummary, type TrackSet } from "@/ipc/commands";
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
 * The inspector's target: the values of the shown file (or the defaults
 * when no file is selected) and an `update` that writes the selected
 * files, or every file when "Apply to" is "All".
 */
export function useInspectorTarget(): InspectorTarget {
  const scope = useJobSettingsStore((state) => state.scope);
  const lead = useQueueStore((queue) => {
    const id = shownItemId(queue);
    return queue.items.find((item) => item.id === id) ?? null;
  });
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
