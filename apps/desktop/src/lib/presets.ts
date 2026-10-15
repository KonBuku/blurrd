/**
 * Presets (plan 6.5): named region, blur, export and verification
 * settings. The built-ins mirror `Preset::builtins` (*Default*, *Max
 * privacy*, *Kill feed only*). The detector options are not part of a
 * preset.
 */
import type { Preset } from "@/bindings/Preset";

import {
  DEFAULT_BLUR,
  DEFAULT_EXPORT,
  DEFAULT_REGIONS,
  DEFAULT_VERIFY,
  type JobSettings,
} from "./job-settings";

/** The settings a preset holds. */
export type PresetValues = Pick<Preset, "regions" | "blur" | "export" | "verify">;

/** Id of the *Default* preset. */
export const DEFAULT_PRESET_ID = "default";

const DEFAULT_PRESET: Preset = {
  id: DEFAULT_PRESET_ID,
  name: "Default",
  builtin: true,
  regions: DEFAULT_REGIONS,
  blur: DEFAULT_BLUR,
  export: DEFAULT_EXPORT,
  verify: DEFAULT_VERIFY,
};

/** The presets that ship with blurrd, in menu order. */
export const BUILTIN_PRESETS: readonly Preset[] = [
  DEFAULT_PRESET,
  {
    ...DEFAULT_PRESET,
    id: "max-privacy",
    name: "Max privacy",
    regions: { ...DEFAULT_REGIONS, mode: "Safe" },
    blur: { ...DEFAULT_BLUR, strength: "High" },
    verify: { ...DEFAULT_VERIFY, coverage: "EveryFrame", audit_output: "EveryFrame" },
  },
  {
    ...DEFAULT_PRESET,
    id: "kill-feed-only",
    name: "Kill feed only",
    regions: { ...DEFAULT_REGIONS, enabled: ["KillFeed"] },
  },
];

/**
 * Applies a preset to settings: regions, blur, export and verification
 * come from the preset, the detector options stay.
 */
export function withPreset(settings: JobSettings, preset: PresetValues): JobSettings {
  return {
    ...settings,
    regions: preset.regions,
    blur: preset.blur,
    export: preset.export,
    verify: preset.verify,
  };
}

/** The preset part of settings (what "Save as preset" stores). */
export function presetValues(settings: JobSettings): PresetValues {
  const { regions, blur, export: output, verify } = settings;
  return { regions, blur, export: output, verify };
}

/** Structural equality of plain JSON values (settings, presets). */
export function sameValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const recordA = a as Record<string, unknown>;
  const recordB = b as Record<string, unknown>;
  const keys = Object.keys(recordA);
  if (keys.length !== Object.keys(recordB).length) return false;
  return keys.every((key) => key in recordB && sameValue(recordA[key], recordB[key]));
}

/** Whether settings differ from a preset in any value the preset holds. */
export function differsFromPreset(settings: JobSettings, preset: PresetValues): boolean {
  const values = presetValues(settings);
  return (["regions", "blur", "export", "verify"] as const).some(
    (key) => !sameValue(values[key], preset[key]),
  );
}

/** A copy without the names kept readable (the default when a preset is shared). */
export function withoutKeptNames<T extends PresetValues>(preset: T): T {
  return {
    ...preset,
    regions: { ...preset.regions, keep_names: { ...preset.regions.keep_names, names: [] } },
  };
}
