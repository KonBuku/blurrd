/**
 * Typed wrappers of the engine commands (plan 9.1). Argument and result
 * types come from the generated bindings; only the saved queue's entry
 * (`SessionEntry`), whose status is a frontend type, is declared here.
 * Every command rejects with an `AppError` (see `ipc/errors.ts`).
 */
import { invoke } from "@tauri-apps/api/core";

import type { Capabilities } from "@/bindings/Capabilities";
import type { DensityLanes } from "@/bindings/DensityLanes";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { FramePts } from "@/bindings/FramePts";
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { OutputPreview } from "@/bindings/OutputPreview";
import type { Override } from "@/bindings/Override";
import type { OverridesSaved } from "@/bindings/OverridesSaved";
import type { ProbeOutcome } from "@/bindings/ProbeOutcome";
import type { ProjectSummary } from "@/bindings/ProjectSummary";
import type { TrackSet } from "@/bindings/TrackSet";
import type { VerifyReport } from "@/bindings/VerifyReport";
import type { RestingStatus } from "@/lib/queue-status";

/** Whether a probe outcome is a usable video. */
export function isProbed(
  outcome: ProbeOutcome,
): outcome is Extract<ProbeOutcome, { media: MediaInfo }> {
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

/** The `open_project` calls in flight, by path. */
const opening = new Map<string, Promise<ProjectSummary>>();

/** Opens (or creates) the project of an analysed or probed file. */
export function openProject(path: string): Promise<ProjectSummary> {
  const opened = invoke<ProjectSummary>("open_project", { path });
  opening.set(path, opened);
  const settled = () => {
    if (opening.get(path) === opened) opening.delete(path);
  };
  opened.then(settled, settled);
  return opened;
}

/** The latest `open_project` call for `path` still in flight, if any. */
export function openingProject(path: string): Promise<ProjectSummary> | undefined {
  return opening.get(path);
}

/**
 * Closes the project of a file removed from the queue: the backend stops
 * serving its video and closes its Exact decoder. Opening the file again
 * opens a new project.
 */
export function closeProject(projectId: string): Promise<void> {
  return invoke<void>("close_project", { projectId });
}

/** The tracks and edits of a project. */
export function getTracks(projectId: string): Promise<TrackSet> {
  return invoke<TrackSet>("get_tracks", { projectId });
}

/** The presentation timestamps of a project's video. */
export function getPts(projectId: string): Promise<FramePts> {
  return invoke<FramePts>("get_pts", { projectId });
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
