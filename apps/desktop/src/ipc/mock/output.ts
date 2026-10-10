/**
 * Output naming and estimates of the mock backend (`get_output_preview`,
 * `start_render`): the template's `{name}` becomes the source's file stem,
 * the extension follows the container, the conflict policy looks at the
 * files earlier mock exports wrote, and an output that is not verified
 * clean gets `_UNVERIFIED` before its extension (plan 6.4, 7.2).
 */
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { OutputPreview } from "@/ipc/commands";
import { pathKey } from "@/lib/media";

import { MOCK_JOB_FPS } from "./jobs";

/** Suffix of outputs that wait for the user to accept their report. */
export const UNVERIFIED_SUFFIX = "_UNVERIFIED";

/** Folder, stem and extension of a Windows path. */
function splitPath(path: string): { folder: string; stem: string; ext: string } {
  const slash = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  const folder = path.slice(0, slash);
  const file = path.slice(slash + 1);
  const dot = file.lastIndexOf(".");
  return dot > 0
    ? { folder, stem: file.slice(0, dot), ext: file.slice(dot) }
    : { folder, stem: file, ext: "" };
}

/** The extension an export writes. */
function extension(settings: ExportSettings, sourceExt: string): string {
  switch (settings.container) {
    case "Source":
      return sourceExt;
    case "Mp4":
      return ".mp4";
    case "Mkv":
      return ".mkv";
    case "Mov":
      return ".mov";
    default:
      return sourceExt;
  }
}

/** Bits per second an export writes (a rough model of the rate control). */
function bitrate(settings: ExportSettings, media: MediaInfo): number {
  const source = media.bitrate ?? 20_000_000;
  const { rate } = settings;
  if ("MatchSource" in rate) return source;
  if ("Vbr" in rate) return rate.Vbr.target_kbps * 1000;
  // Every 6 CQ steps halve the bit rate; CQ 19 is about the source's.
  return source * 2 ** ((19 - rate.Cq) / 6);
}

/**
 * The output of exporting `source` with `settings`: its path (with
 * `_UNVERIFIED` unless `clean`, renamed on conflicts with `existing`) and
 * the size and time estimates.
 */
export function mockOutput(
  source: string,
  media: MediaInfo,
  settings: ExportSettings,
  existing: ReadonlySet<string>,
  clean: boolean,
): OutputPreview {
  const { folder: sourceFolder, stem, ext } = splitPath(source);
  const folder =
    typeof settings.output.folder === "string" ? sourceFolder : settings.output.folder.Folder;
  const base =
    settings.output.template.replaceAll("{name}", stem) + (clean ? "" : UNVERIFIED_SUFFIX);
  const suffix = extension(settings, ext);
  let path = `${folder}\\${base}${suffix}`;
  if (settings.output.on_conflict === "AddNumber") {
    for (let n = 2; existing.has(pathKey(path)); n += 1)
      path = `${folder}\\${base} (${n})${suffix}`;
  }
  const seconds = media.duration_s;
  const audio =
    settings.audio === "Remove"
      ? 0
      : typeof settings.audio === "string"
        ? 192_000
        : settings.audio.Aac * 1000;
  // Slower presets are slower; P4 runs at the measured speed.
  const fps = MOCK_JOB_FPS * (1 + (4 - settings.preset) * 0.15);
  return {
    path,
    est_size: Math.round(((bitrate(settings, media) + audio) * seconds) / 8),
    est_seconds: (media.frame_count ?? seconds * 60) / fps,
  };
}
