/**
 * The app-wide defaults of `settings.json` (plan 6.3 `defaults`,
 * `processing`, `privacy`) and how they meet the per-file job settings:
 * the stored defaults are the settings of files added later (and of files
 * nobody changed); the inspector still overrides them per file.
 */
import type { Defaults } from "@/bindings/Defaults";
import type { Privacy } from "@/bindings/Privacy";
import type { Processing } from "@/bindings/Processing";

import { DEFAULT_JOB_SETTINGS, type JobSettings } from "./job-settings";

/** `Processing::default()`. */
export const DEFAULT_PROCESSING: Processing = {
  gpu_luid: null,
  hw_decode: "Auto",
  encoder: "Auto",
  nvenc_sessions: "Auto",
  parallel_jobs: 1,
  preview_quality: "Auto",
};

/** `Privacy::default()`. */
export const DEFAULT_PRIVACY: Privacy = { log_level: "Info" };

/** `Defaults::default()`: the *Default* preset without detector options. */
export const DEFAULT_DEFAULTS: Defaults = defaultsOf(DEFAULT_JOB_SETTINGS);

/** The stored part of job settings (the detector options are not stored). */
export function defaultsOf(values: JobSettings): Defaults {
  return {
    regions: values.regions,
    blur: values.blur,
    export: values.export,
    verify: values.verify,
  };
}

/** Job settings with the stored defaults, keeping `values`' detector options. */
export function withDefaults(values: JobSettings, defaults: Defaults): JobSettings {
  return { ...values, ...defaults };
}

/** Whether processing runs on the GPU (everything but software decoding). */
export function usesGpu(processing: Processing): boolean {
  return processing.hw_decode !== "Software";
}

/**
 * Whether the saved processing differs from what the running engine uses
 * in a way that needs a restart.
 */
export function needsRestart(saved: Processing, active: Processing): boolean {
  return usesGpu(saved) !== usesGpu(active) || saved.gpu_luid !== active.gpu_luid;
}
