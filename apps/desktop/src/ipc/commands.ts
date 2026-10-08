/**
 * Typed wrappers of the engine commands (plan 9.1). Argument and result
 * types come from the generated bindings where they exist; types the
 * engine does not generate yet are declared here and documented in plan
 * 18.17. Every command rejects with an `AppError` (see `ipc/errors.ts`).
 */
import { invoke } from "@tauri-apps/api/core";

import type { AppError } from "@/bindings/AppError";
import type { Capabilities } from "@/bindings/Capabilities";
import type { MediaInfo } from "@/bindings/MediaInfo";
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
