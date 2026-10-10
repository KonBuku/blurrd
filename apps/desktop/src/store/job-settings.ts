import { create } from "zustand";

import { DEFAULT_JOB_SETTINGS, type JobSettings, type SettingsSection } from "@/lib/job-settings";

import { useQueueStore, type QueueState } from "./queue";

/**
 * Which files an inspector change applies to: the selected files, or
 * every file in the queue (design 8.1 "Apply to").
 */
export type ApplyScope = "selected" | "all";

/** The files one inspector change goes to. */
export interface SettingsTargets {
  /** Queue item ids. */
  readonly ids: readonly string[];
  /** Whether the defaults for files added later change too. */
  readonly defaults: boolean;
}

/** Job settings per queued file (plan 5.9 `project` settings, inspector). */
export interface JobSettingsState {
  /** Settings of files that have none of their own, and of files added later. */
  readonly defaults: JobSettings;
  /** Settings of files changed in the inspector, by queue item id. */
  readonly files: Readonly<Record<string, JobSettings>>;
  readonly scope: ApplyScope;
  readonly setScope: (scope: ApplyScope) => void;
  /**
   * Replaces one section of the targets' settings with `change(section)`.
   * Each target keeps its other values, so changing one field of several
   * files with different settings changes only that field.
   */
  readonly updateSection: <K extends SettingsSection>(
    targets: SettingsTargets,
    key: K,
    change: (section: JobSettings[K]) => JobSettings[K],
  ) => void;
}

/** The settings of a queue item, or the defaults for `null` (no file). */
export function settingsOf(state: JobSettingsState, id: string | null): JobSettings {
  return (id === null ? undefined : state.files[id]) ?? state.defaults;
}

/** Applies `change` to every target. */
function applyTo(
  state: JobSettingsState,
  targets: SettingsTargets,
  change: (settings: JobSettings) => JobSettings,
): Pick<JobSettingsState, "defaults" | "files"> {
  const files = { ...state.files };
  for (const id of targets.ids) files[id] = change(settingsOf(state, id));
  return { defaults: targets.defaults ? change(state.defaults) : state.defaults, files };
}

/**
 * The job settings of every queued file. Files without their own entry use
 * the defaults; a change in the inspector writes the targets' entries (and
 * the defaults when no file is selected or the scope is "all"). Entries of
 * files that leave the queue are dropped.
 */
export const useJobSettingsStore = create<JobSettingsState>()((set) => ({
  defaults: DEFAULT_JOB_SETTINGS,
  files: {},
  scope: "selected",
  setScope: (scope) => set({ scope }),
  updateSection: (targets, key, change) =>
    set((state) =>
      applyTo(state, targets, (settings) => ({ ...settings, [key]: change(settings[key]) })),
    ),
}));

useQueueStore.subscribe((queue, previous) => {
  if (queue.items === previous.items) return;
  const ids = new Set(queue.items.map((item) => item.id));
  const { files } = useJobSettingsStore.getState();
  if (Object.keys(files).every((id) => ids.has(id))) return;
  useJobSettingsStore.setState({
    files: Object.fromEntries(Object.entries(files).filter(([id]) => ids.has(id))),
  });
});

/** Where an inspector change goes for the current queue and scope. */
export function targetsOf(queue: QueueState, scope: ApplyScope): SettingsTargets {
  if (scope === "all") return { ids: queue.items.map((item) => item.id), defaults: true };
  const ids = queue.items.filter((item) => queue.selected.has(item.id)).map((item) => item.id);
  return { ids, defaults: ids.length === 0 };
}
