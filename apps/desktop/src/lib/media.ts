/**
 * Formatting of media facts for the queue and the preview header
 * (design 8.1: "1080p60 · 2:31 · HEVC · 412 MB").
 */
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { Rational } from "@/bindings/Rational";

/** Display names of FFmpeg codec names; others are shown upper-case. */
const CODEC_NAMES: Readonly<Record<string, string>> = {
  h264: "H.264",
  hevc: "HEVC",
  av1: "AV1",
  vp9: "VP9",
  mpeg4: "MPEG-4",
  prores: "ProRes",
};

const BYTE_UNITS = ["B", "KB", "MB", "GB", "TB"] as const;

/** Thousands separators for counts ("1,284"). */
const COUNT = new Intl.NumberFormat("en-US");

/**
 * The last component of a Windows or POSIX path: the file or folder name.
 * Trailing separators are ignored.
 */
export function baseName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const index = Math.max(trimmed.lastIndexOf("\\"), trimmed.lastIndexOf("/"));
  return index === -1 ? trimmed : trimmed.slice(index + 1);
}

/**
 * Whether `path` looks like a file (its name has an extension). Used only
 * for wording while a dropped path is read; the backend decides what the
 * path really is.
 */
export function looksLikeFile(path: string): boolean {
  return /\.[^.\\/]+$/.test(baseName(path));
}

/**
 * A key that is equal for two spellings of the same Windows path: forward
 * slashes become backslashes, letters are compared case-insensitively
 * (NTFS default) and trailing separators are dropped.
 */
export function pathKey(path: string): string {
  return path.replaceAll("/", "\\").replace(/\\+$/, "").toLowerCase();
}

/** Frames per second of a rational, rounded to two decimals ("59.94", "60"). */
export function formatFps([num, den]: Rational): string {
  if (den === 0) return "0";
  return String(Math.round((num / den) * 100) / 100);
}

/** Duration as `m:ss` or `h:mm:ss`, rounded down to whole seconds. */
export function formatDuration(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds));
  const s = total % 60;
  const m = Math.floor(total / 60) % 60;
  const h = Math.floor(total / 3600);
  const ss = String(s).padStart(2, "0");
  return h > 0 ? `${h}:${String(m).padStart(2, "0")}:${ss}` : `${m}:${ss}`;
}

/**
 * A file size with binary multiples and the unit names Windows Explorer
 * uses ("412 MB"); one decimal below 10 units.
 */
export function formatBytes(bytes: number): string {
  let value = Math.max(0, bytes);
  let unit = 0;
  while (value >= 1024 && unit < BYTE_UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  const digits = unit > 0 && value < 10 ? 1 : 0;
  return `${value.toFixed(digits)} ${BYTE_UNITS[unit]}`;
}

/** A count with thousands separators. */
export function formatCount(count: number): string {
  return COUNT.format(count);
}

/** Display name of an FFmpeg codec name ("hevc" → "HEVC"). */
export function codecName(codec: string): string {
  return CODEC_NAMES[codec] ?? codec.toUpperCase();
}

/** Video height and frame rate in the usual short form ("1080p60"). */
export function formatResolution(media: Pick<MediaInfo, "height" | "fps">): string {
  return `${media.height}p${formatFps(media.fps)}`;
}

/** The meta line of a queue item: resolution, duration, codec, size. */
export function mediaSummary(
  media: Pick<MediaInfo, "height" | "fps" | "duration_s" | "video_codec" | "size">,
): string {
  return [
    formatResolution(media),
    formatDuration(media.duration_s),
    codecName(media.video_codec),
    formatBytes(media.size),
  ].join(" · ");
}
