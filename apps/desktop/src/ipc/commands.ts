/**
 * Typed wrappers of the engine commands (plan 9.1). Argument and result
 * types come from the generated bindings where they exist; types the
 * engine does not generate yet are declared here and documented in plan
 * 18.17. Every command rejects with an `AppError` (see `ipc/errors.ts`).
 */
import { invoke } from "@tauri-apps/api/core";

import type { AppError } from "@/bindings/AppError";
import type { MediaInfo } from "@/bindings/MediaInfo";

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
