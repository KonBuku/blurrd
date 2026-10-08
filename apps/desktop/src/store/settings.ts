import { toast } from "sonner";
import { create } from "zustand";

import type { Appearance } from "@/bindings/Appearance";
import { openSettings, type SettingsFile } from "@/ipc/settings";
import { applyAppearance, cacheAppearance, cachedAppearance } from "@/lib/appearance";
import { logWarning } from "@/lib/log";

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
  /** Loads the settings file and applies it. */
  readonly load: () => Promise<void>;
  /** Changes appearance fields, applies them at once and saves them. */
  readonly updateAppearance: (patch: Partial<Appearance>) => void;
}

/** The open settings file, once loaded. */
let file: SettingsFile | null = null;
/** Stops following the system theme of the previous appearance. */
let stopFollowingSystem: () => void = () => undefined;
/** Saves run one after another, so the last change is the one on disk. */
let saving: Promise<void> = Promise.resolve();

/** Applies an appearance to the document and caches it for the next start. */
function apply(appearance: Appearance) {
  stopFollowingSystem();
  stopFollowingSystem = applyAppearance(appearance);
  cacheAppearance(appearance);
}

/**
 * Settings state backed by the settings file. The appearance is applied
 * from the local cache before the first paint (`initialAppearance`), then
 * from the file once it has loaded.
 */
export const useSettingsStore = create<SettingsState>()((set, get) => ({
  status: "loading",
  appearance: cachedAppearance(),

  load: async () => {
    try {
      file = await openSettings();
    } catch (error) {
      logWarning("reading the settings failed", error);
      set({ status: "read-only" });
      return;
    }
    apply(file.appearance);
    set({ appearance: file.appearance, status: file.writable ? "ready" : "read-only" });
  },

  updateAppearance: (patch) => {
    const appearance = { ...get().appearance, ...patch };
    apply(appearance);
    set({ appearance });
    const target = file;
    if (!target?.writable) return;
    saving = saving
      .then(() => target.saveAppearance(appearance))
      .catch((error: unknown) => {
        logWarning("saving the settings failed", error);
        toast.error("Settings could not be saved", {
          description: "The change applies until blurrd closes.",
        });
      });
  },
}));

/** Applies the cached appearance; call once before the first render. */
export function initialAppearance(): void {
  apply(useSettingsStore.getState().appearance);
}
