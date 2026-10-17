/**
 * The settings a job runs with (plan 9.1): which regions are blurred and
 * how the detector works (`start_analysis`), how boxes look and how the
 * file is written (`start_render`), and how the result is verified. The
 * inspector edits one `JobSettings` per queued file; presets and the
 * defaults for new files hold the same shape.
 */
import type { BlurParams } from "@/bindings/BlurParams";
import type { DetectorOptions } from "@/bindings/DetectorOptions";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { Region } from "@/bindings/Region";
import type { RegionSettings } from "@/bindings/RegionSettings";
import type { VerifySettings } from "@/bindings/VerifySettings";

import { REGION_ORDER } from "./regions";

/** Everything a job of one file runs with. */
export interface JobSettings {
  readonly regions: RegionSettings;
  readonly detector: DetectorOptions;
  readonly blur: BlurParams;
  readonly export: ExportSettings;
  readonly verify: VerifySettings;
}

/** A section of {@link JobSettings}. */
export type SettingsSection = keyof JobSettings;

/** A region the detector finds (every region but manual boxes). */
export type DetectedRegion = Exclude<Region, "Manual">;

/** Detector regions every new file starts with: all of them (D8). */
export const ALL_REGIONS: readonly DetectedRegion[] = REGION_ORDER.filter(
  (region): region is DetectedRegion => region !== "Manual",
);

/** `RegionSettings::default()`: every region enabled (D8). */
export const DEFAULT_REGIONS: RegionSettings = {
  enabled: [...ALL_REGIONS],
  mode: "Tight",
  keep_names: { own_row: false, names: [] },
  chat_blur_message: false,
  chat_blur_system: true,
};

/** `TemporalParams::default()` and the balanced sensitivity. */
export const DEFAULT_DETECTOR: DetectorOptions = {
  sensitivity: "Balanced",
  temporal: { pre_roll: 3, post_roll_min: 18, max_gap: 45 },
};

/** `BlurParams::default()`. */
export const DEFAULT_BLUR: BlurParams = {
  style: "PixelBlur",
  strength: "Medium",
  pad: [4, 3],
  radius: 6,
  feather: 1.5,
  noise: true,
};

/** `ExportSettings::default()` (plan 6.4). */
export const DEFAULT_EXPORT: ExportSettings = {
  codec: "Source",
  encoder: "Auto",
  rate: { MatchSource: { max_factor: 1.5 } },
  preset: 3,
  ten_bit: "Auto",
  split_encode: "Auto",
  nvenc_sessions: "Auto",
  keyframes: "MatchSource",
  smart_render: false,
  audio: "Copy",
  copy_subtitles: true,
  metadata: "Technical",
  container: "Source",
  moov: "Reserve",
  trim: null,
  output: { folder: "NextToSource", template: "{name}_blurrd", on_conflict: "AddNumber" },
};

/** `VerifySettings::default()`. */
export const DEFAULT_VERIFY: VerifySettings = {
  enabled: true,
  mode: "DetectAndFix",
  coverage: "Structure",
  full_frame_sample_ms: 500,
  min_conf: 0.5,
  audit_output: "Off",
  ignore_words: [],
};

/** The settings of a file nobody changed: the *Default* preset. */
export const DEFAULT_JOB_SETTINGS: JobSettings = {
  regions: DEFAULT_REGIONS,
  detector: DEFAULT_DETECTOR,
  blur: DEFAULT_BLUR,
  export: DEFAULT_EXPORT,
  verify: DEFAULT_VERIFY,
};

/** Enabled regions in inspector order, without duplicates. */
export function orderedRegions(regions: Iterable<Region>): DetectedRegion[] {
  const set = new Set<Region>(regions);
  return ALL_REGIONS.filter((region) => set.has(region));
}

/** Splits a typed word list ("ACE, CLUTCH; Haven") into distinct trimmed words. */
export function parseWordList(text: string): string[] {
  const words = text
    .split(/[,;\n]/)
    .map((word) => word.trim())
    .filter((word) => word.length > 0);
  return [...new Set(words)];
}
