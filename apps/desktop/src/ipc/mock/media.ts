/**
 * A fake file system and `probe_media` for the mock backend: video files
 * get deterministic media facts derived from their name, folders expand
 * to the videos registered for them, everything else fails like the real
 * probe would.
 */
import type { AppError } from "@/bindings/AppError";
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { ProbeOutcome } from "@/bindings/ProbeOutcome";
import { baseName, pathKey } from "@/lib/media";

/** Extensions the fake probe accepts as videos. */
const VIDEO = /\.(mp4|mov|mkv)$/i;

/** Source formats the fake media cycle through: 1080p, 1440p, 720p, 4:3 and 4K. */
const FORMATS = [
  { width: 1920, height: 1080, fps: [60, 1], codec: "h264" },
  { width: 2560, height: 1440, fps: [144, 1], codec: "hevc" },
  { width: 1280, height: 720, fps: [30, 1], codec: "h264" },
  { width: 1440, height: 1080, fps: [60000, 1001], codec: "hevc" },
  { width: 3840, height: 2160, fps: [60, 1], codec: "av1" },
] as const;

/** Folders of the default fake file system: folder path → files in it. */
export const DEFAULT_FOLDERS: ReadonlyMap<string, readonly string[]> = new Map([
  [
    "D:\\Recordings\\Ranked",
    [
      "D:\\Recordings\\Ranked\\ascent_ranked_01.mp4",
      "D:\\Recordings\\Ranked\\bind_ranked_02.mp4",
      "D:\\Recordings\\Ranked\\haven_ranked_03.mkv",
      "D:\\Recordings\\Ranked\\notes.txt",
    ],
  ],
]);

/** What the file dialog returns by default in `pnpm dev:mock`. */
export const DEFAULT_PICK: readonly string[] = [
  "D:\\Recordings\\match_ascent_01.mp4",
  "D:\\Recordings\\match_bind_02.mp4",
  "D:\\Recordings\\match_lotus_04.mov",
  "D:\\Recordings\\thumbnail.png",
];

/** A small stable hash of a string (FNV-1a), for deterministic fake facts. */
function hash(text: string): number {
  let value = 0x811c9dc5;
  for (const char of text) {
    value ^= char.codePointAt(0) ?? 0;
    value = Math.imul(value, 0x01000193) >>> 0;
  }
  return value;
}

function error(kind: AppError["kind"], message: string): AppError {
  return { kind, message, detail: null };
}

/** Fake media facts of a video file. */
export function fakeMedia(path: string): MediaInfo {
  const seed = hash(baseName(path).toLowerCase());
  const format = FORMATS[seed % FORMATS.length] ?? FORMATS[0];
  const duration = 20 + (seed % 900);
  const bitrate = 12_000_000 + (seed % 40) * 1_000_000;
  const [num, den] = format.fps;
  return {
    path,
    size: Math.round((duration * bitrate) / 8),
    duration_s: duration,
    width: format.width,
    height: format.height,
    fps: [num, den],
    time_base: [1, 15360],
    frame_count: Math.round((duration * num) / den),
    video_codec: format.codec,
    profile: format.codec === "h264" ? "High" : "Main",
    bit_depth: 8,
    color: { primaries: "bt709", transfer: "bt709", matrix: "bt709", range: "Limited" },
    hdr: null,
    has_b_frames: false,
    has_edit_list: false,
    keyframes: null,
    bitrate,
    audio: [
      { index: 1, codec: "aac", channels: 2, sample_rate: 48000, bitrate: 192000, language: null },
    ],
    has_subtitles: false,
    is_vfr: false,
  };
}

/** Probes one file the way the real backend would answer. */
function probeFile(path: string): ProbeOutcome {
  const name = baseName(path).toLowerCase();
  if (name.includes("missing"))
    return { path, error: error("NotFound", "The file does not exist.") };
  if (!VIDEO.test(name)) {
    return { path, error: error("UnsupportedMedia", "This file is not a supported video.") };
  }
  if (name.includes("corrupt")) {
    return { path, error: error("Decode", "The video stream could not be decoded.") };
  }
  return { path, media: fakeMedia(path) };
}

/**
 * `probe_media`: folders expand to the videos registered for them (other
 * files skipped, sorted by name); paths named directly get one outcome.
 */
export function probeMedia(
  paths: readonly string[],
  folders: ReadonlyMap<string, readonly string[]>,
): ProbeOutcome[] {
  const byKey = new Map([...folders].map(([folder, files]) => [pathKey(folder), files]));
  return paths.flatMap((path) => {
    const files = byKey.get(pathKey(path));
    if (files === undefined) return [probeFile(path)];
    return files
      .filter((file) => VIDEO.test(file))
      .toSorted((a, b) => a.localeCompare(b))
      .map(probeFile);
  });
}
