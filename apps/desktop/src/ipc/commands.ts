/**
 * Typed wrappers of the engine commands (plan 9.1). Argument and result
 * types come from the generated bindings where they exist; types the
 * engine does not generate yet are declared here and documented in plan
 * section 18 (M23, M24 and M25 decisions). Every command rejects with an
 * `AppError` (see `ipc/errors.ts`).
 */
import { invoke } from "@tauri-apps/api/core";

import type { AppError } from "@/bindings/AppError";
import type { BlurParams } from "@/bindings/BlurParams";
import type { Capabilities } from "@/bindings/Capabilities";
import type { DensityLanes } from "@/bindings/DensityLanes";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { FrameRange } from "@/bindings/FrameRange";
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { Override } from "@/bindings/Override";
import type { OverrideBinding } from "@/bindings/OverrideBinding";
import type { ProjectVerify } from "@/bindings/ProjectVerify";
import type { Rational } from "@/bindings/Rational";
import type { Region } from "@/bindings/Region";
import type { RegionSet } from "@/bindings/RegionSet";
import type { Track } from "@/bindings/Track";
import type { VerifyReport } from "@/bindings/VerifyReport";
import type { RestingStatus } from "@/lib/queue-status";

/** Result of probing one file: its media facts or why it cannot be used. */
export type ProbeOutcome =
  | { readonly path: string; readonly media: MediaInfo }
  | { readonly path: string; readonly error: AppError };

/** Whether a probe outcome is a usable video. */
export function isProbed(
  outcome: ProbeOutcome,
): outcome is { readonly path: string; readonly media: MediaInfo } {
  return "media" in outcome;
}

/**
 * Reads the media facts of dropped or picked paths. A folder expands to
 * the videos inside it (sorted by name, other files skipped); every file
 * named directly gets an outcome, an error when it is not a usable video.
 */
export function probeMedia(paths: readonly string[]): Promise<ProbeOutcome[]> {
  return invoke<ProbeOutcome[]>("probe_media", { paths });
}

/**
 * GPUs, decoders, encoders and OCR of this machine. Answers at once;
 * `probe_state` stays `Pending` until the background probe has finished,
 * so callers poll until it is `Ready`.
 */
export function getCapabilities(): Promise<Capabilities> {
  return invoke<Capabilities>("get_capabilities");
}

/**
 * One file of the saved queue (`session.json`, plan 6.2). The backend adds
 * the project fingerprint when it writes the file.
 */
export interface SessionEntry {
  readonly path: string;
  /** Status when the queue was saved; running jobs are saved as resting. */
  readonly status: RestingStatus;
  /** The exported file, once there is one. */
  readonly output: string | null;
}

/** The queue saved by the previous run, in queue order. */
export function getSession(): Promise<SessionEntry[]> {
  return invoke<SessionEntry[]>("get_session");
}

/** Replaces the saved queue. */
export function saveSession(entries: readonly SessionEntry[]): Promise<void> {
  return invoke<void>("save_session", { entries });
}

/**
 * An opened project as the preview needs it (`open_project`, plan 9.1).
 * Declared here until the engine generates it (plan section 18, M24).
 */
export interface ProjectSummary {
  /** Project id: the source fingerprint, used in `blurrd://` URLs. */
  readonly id: string;
  readonly media: MediaInfo;
  /** URL of the source video (asset protocol or the loopback server). */
  readonly media_url: string;
  /** URL of the H.264 proxy made during analysis, if there is one. */
  readonly proxy_url: string | null;
  /** HUD scale of the video (`HudGeometry::sy`), for the blur sizes. */
  readonly sy: number;
  readonly blur: BlurParams;
  /** Regions the blur applies to (Manual boxes always apply). */
  readonly enabled: RegionSet;
  /** Whether an analysis has produced tracks. */
  readonly analyzed: boolean;
  readonly verify: ProjectVerify;
  /** Timeline thumbnails: one every `every` frames, `count` in total. */
  readonly thumbs: { readonly every: number; readonly count: number } | null;
}

/** A span whose region detection was unhealthy (`RegionHealth` warning). */
export interface HealthSpan {
  readonly region: Region;
  /** First and last frame (inclusive). */
  readonly f0: number;
  readonly f1: number;
}

/** Tracks, user edits and their binding state (`get_tracks`). */
export interface TrackSet {
  readonly revision: number;
  readonly tracks: readonly Track[];
  readonly overrides: readonly Override[];
  readonly bindings: readonly OverrideBinding[];
  /** Spans where a region fell back to its safe slot boxes. */
  readonly health: readonly HealthSpan[];
}

/**
 * Presentation timestamps of every decoded frame, in `time_base` units
 * (`pts.bin`, plan 6.2); index = presentation frame index.
 */
export interface PtsTable {
  readonly time_base: Rational;
  readonly pts: readonly number[];
}

/** Opens (or creates) the project of an analysed or probed file. */
export function openProject(path: string): Promise<ProjectSummary> {
  return invoke<ProjectSummary>("open_project", { path });
}

/** The tracks and edits of a project. */
export function getTracks(projectId: string): Promise<TrackSet> {
  return invoke<TrackSet>("get_tracks", { projectId });
}

/** The presentation timestamps of a project's video. */
export function getPts(projectId: string): Promise<PtsTable> {
  return invoke<PtsTable>("get_pts", { projectId });
}

/** Per-region box coverage in `buckets` equal parts of the timeline. */
export function getTimelineDensity(projectId: string, buckets: number): Promise<DensityLanes> {
  return invoke<DensityLanes>("get_timeline_density", { projectId, buckets });
}

/** The last verification report, or `null` before the first one. */
export function getVerifyReport(projectId: string): Promise<VerifyReport | null> {
  return invoke<VerifyReport | null>("get_verify_report", { projectId });
}

/**
 * What `set_overrides` answers: the new project revision and the frame
 * ranges that edits changed since the last verification (verified again
 * before export).
 */
export interface OverridesSaved {
  readonly revision: number;
  readonly unverified_ranges: readonly FrameRange[];
}

/**
 * Replaces the edits of a project with `overrides` (the full list). The
 * backend rejects the call with a `StaleRevision` error when the project
 * changed since `baseRev` (another save, a re-analysis); the caller then
 * reloads.
 */
export function setOverrides(
  projectId: string,
  baseRev: number,
  overrides: readonly Override[],
): Promise<OverridesSaved> {
  return invoke<OverridesSaved>("set_overrides", { projectId, baseRev, overrides });
}

/**
 * Where an export with given settings would go and what it would cost
 * (`get_output_preview`, plan 9.1): the output path after the naming
 * template and the conflict policy, the estimated size in bytes and the
 * estimated duration in seconds at this machine's measured throughput.
 * Declared here until the engine generates it (plan section 18, M26).
 */
export interface OutputPreview {
  readonly path: string;
  readonly est_size: number;
  readonly est_seconds: number;
}

/** The output path and estimates of exporting a project with `settings`. */
export function getOutputPreview(
  projectId: string,
  settings: ExportSettings,
): Promise<OutputPreview> {
  return invoke<OutputPreview>("get_output_preview", { projectId, settings });
}

/**
 * Accepts the verification report `reportId` of a project (its open leaks
 * become accepted) and renames the `_UNVERIFIED` output; answers the new
 * output path (plan 7.2).
 */
export function acceptReport(projectId: string, reportId: string): Promise<string> {
  return invoke<string>("accept_report", { projectId, reportId });
}
