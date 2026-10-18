import { toast } from "sonner";
import { create } from "zustand";

import type { Appearance } from "@/bindings/Appearance";
import type { Defaults } from "@/bindings/Defaults";
import type { General } from "@/bindings/General";
import type { Privacy } from "@/bindings/Privacy";
import type { Processing } from "@/bindings/Processing";
import { isAppError } from "@/ipc/errors";
import {
  getStoredSettings,
  openSettings,
  saveDefaults,
  savePrivacy,
  saveProcessing,
  type SettingsFile,
} from "@/ipc/settings";
import { DEFAULT_PRIVACY, DEFAULT_PROCESSING, defaultsOf } from "@/lib/app-defaults";
import { applyAppearance, cacheAppearance, cachedAppearance } from "@/lib/appearance";
import { DEFAULT_GENERAL } from "@/lib/general";
import { logWarning } from "@/lib/log";

import { useJobSettingsStore } from "./job-settings";

/** Where the settings come from. */
export type SettingsStatus =
  /** The first paint uses the cached copy; the file is still loading. */
  | "loading"
  /** Loaded from the settings file; changes are saved. */
  | "ready"
  /** The file cannot be written; changes last until the app closes. */
  | "read-only";

/** App settings shown on the settings page. */
export interface SettingsState {
  readonly status: SettingsStatus;
  readonly appearance: Appearance;
  readonly general: General;
  /** Processing as saved (the engine uses it from the next start). */
  readonly processing: Processing;
  /** Processing as the running engine uses it. */
  readonly activeProcessing: Processing;
  readonly privacy: Privacy;
  /** Loads the settings file and applies it. */
  readonly load: () => Promise<void>;
  /** Changes appearance fields, applies them at once and saves them. */
  readonly updateAppearance: (patch: Partial<Appearance>) => void;
  /** Changes general fields and saves them. */
  readonly updateGeneral: (patch: Partial<General>) => void;
  /** Changes processing fields and saves them (checked by the backend). */
  readonly updateProcessing: (patch: Partial<Processing>) => void;
  /** Changes privacy fields and saves them; the log level applies at once. */
  readonly updatePrivacy: (patch: Partial<Privacy>) => void;
}

/** The open settings file, once loaded. */
let file: SettingsFile | null = null;
/** Whether the backend may write its sections (same rule as the file). */
let backendWritable = false;
/** Stops following the system theme of the previous appearance. */
let stopFollowingSystem: () => void = () => undefined;
/** Saves run one after another, so the last change is the one on disk. */
let saving: Promise<void> = Promise.resolve();
/** Settle time of the defaults for new files before they are saved. */
const DEFAULTS_DEBOUNCE_MS = 300;

/** Queues a save after the previous ones; a failure is logged and shown once. */
function enqueue(write: () => Promise<unknown>) {
  saving = saving
    .then(async () => {
      await write();
    })
    .catch((error: unknown) => {
      logWarning("saving the settings failed", error);
      toast.error("Settings could not be saved", {
        description: isAppError(error) ? error.message : "The change applies until blurrd closes.",
      });
    });
}

/** Saves a section the frontend writes itself. */
function persist(write: (target: SettingsFile) => Promise<void>) {
  const target = file;
  if (!target?.writable) return;
  enqueue(() => write(target));
}

/** Saves a section the backend checks and writes. */
function persistBackend(write: () => Promise<unknown>) {
  if (backendWritable) enqueue(write);
}

/** Applies an appearance to the document and caches it for the next start. */
function apply(appearance: Appearance) {
  stopFollowingSystem();
  stopFollowingSystem = applyAppearance(appearance);
  cacheAppearance(appearance);
}

/** The defaults last saved or loaded, to skip saves without a change. */
let savedDefaults = "";
let defaultsTimer: ReturnType<typeof setTimeout> | undefined;
let stopWatchingDefaults: (() => void) | null = null;

/**
 * Saves the defaults for new files a moment after they stop changing,
 * whether a settings page or the inspector (no file selected, or "Apply
 * to all") changed them.
 */
function watchDefaults() {
  stopWatchingDefaults?.();
  stopWatchingDefaults = useJobSettingsStore.subscribe((state, previous) => {
    if (state.defaults.values === previous.defaults.values) return;
    clearTimeout(defaultsTimer);
    defaultsTimer = setTimeout(() => {
      const defaults = defaultsOf(useJobSettingsStore.getState().defaults.values);
      const text = JSON.stringify(defaults);
      if (text === savedDefaults) return;
      savedDefaults = text;
      persistBackend(() => saveDefaults(defaults));
    }, DEFAULTS_DEBOUNCE_MS);
  });
}

/** Makes the stored `defaults` the defaults for new files. */
function loadDefaults(defaults: Defaults) {
  savedDefaults = JSON.stringify(defaults);
  useJobSettingsStore.getState().loadDefaults(defaults);
}

/**
 * Settings state backed by the settings file. The appearance is applied
 * from the local cache before the first paint (`initialAppearance`), then
 * from the file once it has loaded. Processing, privacy and the defaults
 * for new files (held by the job settings store) come from the backend,
 * which checks every value before it writes it.
 */
export const useSettingsStore = create<SettingsState>()((set, get) => ({
  status: "loading",
  appearance: cachedAppearance(),
  general: DEFAULT_GENERAL,
  processing: DEFAULT_PROCESSING,
  activeProcessing: DEFAULT_PROCESSING,
  privacy: DEFAULT_PRIVACY,

  load: async () => {
    try {
      file = await openSettings();
    } catch (error) {
      logWarning("reading the settings failed", error);
      set({ status: "read-only" });
      return;
    }
    apply(file.appearance);
    set({
      appearance: file.appearance,
      general: file.general,
      status: file.writable ? "ready" : "read-only",
    });
    try {
      const stored = await getStoredSettings();
      backendWritable = stored.writable;
      loadDefaults(stored.defaults);
      set({
        processing: stored.processing,
        activeProcessing: stored.active_processing,
        privacy: stored.privacy,
      });
    } catch (error) {
      backendWritable = false;
      logWarning("reading the stored defaults failed", error);
    }
    watchDefaults();
  },

  updateAppearance: (patch) => {
    const appearance = { ...get().appearance, ...patch };
    apply(appearance);
    set({ appearance });
    persist((target) => target.saveAppearance(appearance));
  },

  updateGeneral: (patch) => {
    const general = { ...get().general, ...patch };
    set({ general });
    persist((target) => target.saveGeneral(general));
  },

  updateProcessing: (patch) => {
    const processing = { ...get().processing, ...patch };
    set({ processing });
    persistBackend(() => saveProcessing(processing));
  },

  updatePrivacy: (patch) => {
    const privacy = { ...get().privacy, ...patch };
    set({ privacy });
    persistBackend(() => savePrivacy(privacy));
  },
}));

/** Applies the cached appearance; call once before the first render. */
export function initialAppearance(): void {
  apply(useSettingsStore.getState().appearance);
}
