import { create } from "zustand";

import type { Defaults } from "@/bindings/Defaults";
import type { Preset } from "@/bindings/Preset";
import { withDefaults } from "@/lib/app-defaults";
import { DEFAULT_JOB_SETTINGS, type JobSettings, type SettingsSection } from "@/lib/job-settings";
import { DEFAULT_PRESET_ID, withPreset } from "@/lib/presets";

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

/** The settings of one file, the preset they are named after and where they started. */
export interface FileSettings {
  /** Id of the preset applied last (the *Default* preset for the app defaults). */
  readonly presetId: string;
  readonly values: JobSettings;
  /**
   * The values the file started from: the preset applied last, or the app
   * defaults (Settings) for files nobody changed. Values that differ from
   * them show as "modified".
   */
  readonly base: JobSettings;
}

/** Job settings per queued file (plan 5.9 `project` settings, inspector). */
export interface JobSettingsState {
  /** Settings of files that have none of their own, and of files added later. */
  readonly defaults: FileSettings;
  /** Settings of files changed in the inspector, by queue item id. */
  readonly files: Readonly<Record<string, FileSettings>>;
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
  /** Applies a preset to the targets. */
  readonly applyPreset: (targets: SettingsTargets, preset: Preset) => void;
  /** Puts each target back to the values it started from. */
  readonly resetToBase: (targets: SettingsTargets) => void;
  /**
   * Changes one section of the defaults for new files (Settings › Output
   * defaults, Detection, Verification): they are the starting values of
   * files added from now on. Queued files that used the defaults keep
   * their current values.
   */
  readonly updateDefaults: <K extends SettingsSection>(
    key: K,
    change: (section: JobSettings[K]) => JobSettings[K],
  ) => void;
  /** Replaces the defaults with the stored ones (at start). */
  readonly loadDefaults: (defaults: Defaults) => void;
}

/** The settings entry of a queue item, or the defaults for `null` (no file). */
export function fileSettingsOf(state: JobSettingsState, id: string | null): FileSettings {
  return (id === null ? undefined : state.files[id]) ?? state.defaults;
}

/** The settings of a queue item, or the defaults for `null` (no file). */
export function settingsOf(state: JobSettingsState, id: string | null): JobSettings {
  return fileSettingsOf(state, id).values;
}

/** Applies `change` to every target. */
function applyTo(
  state: JobSettingsState,
  targets: SettingsTargets,
  change: (entry: FileSettings) => FileSettings,
): Pick<JobSettingsState, "defaults" | "files"> {
  const files = { ...state.files };
  for (const id of targets.ids) files[id] = change(fileSettingsOf(state, id));
  return { defaults: targets.defaults ? change(state.defaults) : state.defaults, files };
}

/**
 * The job settings of every queued file. Files without their own entry use
 * the defaults; a change in the inspector writes the targets' entries (and
 * the defaults when no file is selected or the scope is "all"). Entries of
 * files that leave the queue are dropped.
 */
export const useJobSettingsStore = create<JobSettingsState>()((set) => ({
  defaults: {
    presetId: DEFAULT_PRESET_ID,
    values: DEFAULT_JOB_SETTINGS,
    base: DEFAULT_JOB_SETTINGS,
  },
  files: {},
  scope: "selected",
  setScope: (scope) => set({ scope }),
  updateSection: (targets, key, change) =>
    set((state) =>
      applyTo(state, targets, (entry) => ({
        ...entry,
        values: { ...entry.values, [key]: change(entry.values[key]) },
      })),
    ),
  applyPreset: (targets, preset) =>
    set((state) =>
      applyTo(state, targets, (entry) => {
        const values = withPreset(entry.values, preset);
        return { presetId: preset.id, values, base: values };
      }),
    ),
  resetToBase: (targets) =>
    set((state) => applyTo(state, targets, (entry) => ({ ...entry, values: entry.base }))),
  updateDefaults: (key, change) =>
    set((state) => {
      const files = { ...state.files };
      for (const item of useQueueStore.getState().items) files[item.id] ??= state.defaults;
      const { values } = state.defaults;
      const next = { ...values, [key]: change(values[key]) };
      return { files, defaults: { ...state.defaults, values: next, base: next } };
    }),
  loadDefaults: (defaults) =>
    set((state) => {
      const values = withDefaults(state.defaults.values, defaults);
      return { defaults: { ...state.defaults, values, base: values } };
    }),
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
