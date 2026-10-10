/**
 * What the export inspector offers on this machine: which codecs and
 * encoders work (from `get_capabilities`), which options apply to the
 * chosen codec, the naming template rules and the one-line summary the
 * export footer shows.
 */
import type { Capabilities } from "@/bindings/Capabilities";
import type { Codec } from "@/bindings/Codec";
import type { EncoderInfo } from "@/bindings/EncoderInfo";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { RateControl } from "@/bindings/RateControl";
import type { VerifySettings } from "@/bindings/VerifySettings";

import { codecName } from "./media";

/** The codecs the user can choose besides "Same as source". */
export const OUTPUT_CODECS = ["H264", "Hevc", "Av1"] as const satisfies readonly Codec[];

/** FFmpeg codec name of an output codec. */
const FFMPEG_CODEC = { H264: "h264", Hevc: "hevc", Av1: "av1" } as const;

/** Display name of a codec choice. */
export const CODEC_LABELS: Readonly<Record<Codec, string>> = {
  Source: "Same as source",
  H264: "H.264",
  Hevc: "HEVC",
  Av1: "AV1",
};

/** Default constant quality per codec (design 8.1: H.264 19, HEVC 21, AV1 28). */
export const DEFAULT_CQ: Readonly<Record<Exclude<Codec, "Source">, number>> = {
  H264: 19,
  Hevc: 21,
  Av1: 28,
};

/** The FFmpeg codec an export writes: the chosen one, or the source's. */
export function effectiveCodec(codec: Codec, sourceCodec: string | null): string | null {
  return codec === "Source" ? sourceCodec : FFMPEG_CODEC[codec];
}

/** The output codec choice that writes the FFmpeg codec `name`, if any. */
function codecOf(name: string): Exclude<Codec, "Source"> | null {
  return OUTPUT_CODECS.find((codec) => FFMPEG_CODEC[codec] === name) ?? null;
}

/** Working encoders of an FFmpeg codec, hardware first. */
export function encodersFor(caps: Capabilities | null, codec: string | null): EncoderInfo[] {
  if (!caps || codec === null) return [];
  return caps.encoders
    .filter((encoder) => encoder.available && encoder.codec === codec)
    .toSorted((a, b) => Number(b.hardware) - Number(a.hardware));
}

/**
 * Why a codec cannot be chosen, or `null` when it can: the encoder probe
 * is still running, or no working encoder exists for it.
 */
export function codecProblem(caps: Capabilities | null, codec: Codec): string | null {
  if (codec === "Source") return null;
  if (!caps || caps.probe_state === "Pending") return "Checking the encoders of this PC…";
  if (encodersFor(caps, FFMPEG_CODEC[codec]).length > 0) return null;
  const name = CODEC_LABELS[codec];
  return codec === "Av1"
    ? `${name} is not available: this PC has no working AV1 encoder (NVIDIA needs a GeForce RTX 40 series or newer).`
    : `${name} is not available: this PC has no working ${name} encoder.`;
}

/** A readable encoder name: "NVENC (GPU)", "x264 (CPU)", … */
export function encoderLabel(name: string): string {
  const vendor = name.split("_")[1];
  if (vendor === "nvenc") return "NVENC (GPU)";
  if (vendor === "amf") return "AMF (GPU)";
  if (vendor === "qsv") return "Quick Sync (GPU)";
  if (vendor === "mf") return "Media Foundation";
  if (name === "libx264") return "x264 (CPU)";
  if (name === "libx265") return "x265 (CPU)";
  if (name === "libsvtav1") return "SVT-AV1 (CPU)";
  return name;
}

/** The encoder an export uses: the named one if it works, else the best working one. */
export function chosenEncoder(
  caps: Capabilities | null,
  settings: ExportSettings,
  sourceCodec: string | null,
): EncoderInfo | null {
  const candidates = encodersFor(caps, effectiveCodec(settings.codec, sourceCodec));
  const { encoder } = settings;
  if (typeof encoder !== "string") {
    const named = candidates.find((candidate) => candidate.name === encoder.Named);
    if (named) return named;
  }
  return candidates[0] ?? null;
}

/**
 * Why 10-bit output cannot be forced on, or `null` when it can: only
 * HEVC and AV1 carry it, and only with an encoder that writes it.
 */
export function tenBitProblem(
  caps: Capabilities | null,
  settings: ExportSettings,
  sourceCodec: string | null,
): string | null {
  const codec = effectiveCodec(settings.codec, sourceCodec);
  if (codec === null || codecOf(codec) === "H264" || codecOf(codec) === null) {
    return "10-bit output needs HEVC or AV1.";
  }
  const encoder = chosenEncoder(caps, settings, sourceCodec);
  return encoder && !encoder.ten_bit
    ? `${encoderLabel(encoder.name)} does not write 10-bit video.`
    : null;
}

/** Characters Windows does not allow in file names (besides control characters). */
const INVALID_NAME = /[<>:"/\\|?*]/;

/** Whether text contains a control character (U+0000 to U+001F). */
function hasControl(text: string): boolean {
  return [...text].some((char) => (char.codePointAt(0) ?? 0) < 0x20);
}

/** What is wrong with a file name template, or `null` when it is fine. */
export function templateProblem(template: string): string | null {
  const trimmed = template.trim();
  if (trimmed === "") return "Enter a file name.";
  if (INVALID_NAME.test(trimmed) || hasControl(trimmed)) {
    return 'A file name cannot contain < > : " / \\ | ? * or control characters.';
  }
  if (/[. ]$/.test(trimmed)) return "A file name cannot end with a dot or a space.";
  return null;
}

/** Short form of a rate control: "Match source", "CQ 19", "30 Mbit/s". */
export function rateText(rate: RateControl): string {
  if ("MatchSource" in rate) return "match source bit rate";
  if ("Cq" in rate) return `CQ ${rate.Cq}`;
  return `${formatMbps(rate.Vbr.target_kbps)} Mbit/s`;
}

/** kbit/s as Mbit/s with at most one decimal. */
export function formatMbps(kbps: number): string {
  return String(Math.round(kbps / 100) / 10);
}

/**
 * The export summary under the footer button (design 8.1): codec and
 * encoder, rate control, audio and whether verification runs, for
 * example "H.264 NVENC · CQ 19 · audio copy · verify on".
 */
export function exportSummary(
  settings: ExportSettings,
  verify: VerifySettings,
  encoder: EncoderInfo | null,
  sourceCodec: string | null,
): string {
  const codec = effectiveCodec(settings.codec, sourceCodec);
  const video = [
    codec === null ? "Same codec" : codecName(codec),
    encoder ? encoderLabel(encoder.name).replace(/ \((GPU|CPU)\)$/, "") : null,
  ]
    .filter((part) => part !== null)
    .join(" ");
  const audio =
    settings.audio === "Copy"
      ? "audio copy"
      : settings.audio === "Remove"
        ? "no audio"
        : `AAC ${settings.audio.Aac} kbit/s`;
  return [video, rateText(settings.rate), audio, verify.enabled ? "verify on" : "verify off"].join(
    " · ",
  );
}
