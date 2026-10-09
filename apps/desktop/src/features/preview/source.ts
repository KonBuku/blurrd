/**
 * Which stream the preview plays: the source itself when WebView2 can
 * decode its codec, otherwise the H.264 proxy made during analysis (plan
 * C7, C15). HEVC needs the HEVC Video Extensions on Windows and AV1 a
 * recent GPU or the AV1 extension; the codec check avoids a failed load,
 * and a decode error at runtime falls back the same way.
 */
import type { ProjectSummary } from "@/ipc/commands";

/** A MIME type with a typical codec string per FFmpeg codec name. */
const CODEC_TYPES: Readonly<Record<string, string>> = {
  h264: 'video/mp4; codecs="avc1.640028"',
  hevc: 'video/mp4; codecs="hvc1.1.6.L120.90"',
  av1: 'video/mp4; codecs="av01.0.08M.08"',
  vp9: 'video/webm; codecs="vp09.00.40.08"',
};

/** What the browser says about a MIME type (`HTMLMediaElement.canPlayType`). */
export type CanPlay = (type: string) => CanPlayTypeResult;

/** The stream the preview plays. */
export interface PreviewSource {
  readonly url: string;
  /** Whether this is the proxy (smaller than the source). */
  readonly proxy: boolean;
}

/**
 * The stream to start with: the source unless the browser reports that it
 * cannot decode the codec and a proxy exists. Unknown codecs are tried.
 */
export function initialSource(project: ProjectSummary, canPlay: CanPlay): PreviewSource {
  const type = CODEC_TYPES[project.media.video_codec];
  const playable = type === undefined || canPlay(type) !== "";
  if (!playable && project.proxy_url !== null) return { url: project.proxy_url, proxy: true };
  return { url: project.media_url, proxy: false };
}

/**
 * The stream to switch to after `current` failed to load or decode, or
 * `null` when there is nothing left to try.
 */
export function fallbackSource(
  project: ProjectSummary,
  current: PreviewSource,
): PreviewSource | null {
  if (current.proxy || project.proxy_url === null) return null;
  return { url: project.proxy_url, proxy: true };
}

/** The explanation shown when neither the source nor a proxy plays. */
export function problemText(project: ProjectSummary): string {
  const codec = project.media.video_codec;
  if (codec === "hevc") {
    return "This HEVC video needs the HEVC Video Extensions from the Microsoft Store to play here. Analyzing it creates a preview copy that plays without them.";
  }
  if (codec === "av1") {
    return "This AV1 video needs the AV1 Video Extension or a GPU that decodes AV1. Analyzing it creates a preview copy that plays without it.";
  }
  return "This video can't be played in the preview. Analyzing it creates a preview copy that plays here.";
}
