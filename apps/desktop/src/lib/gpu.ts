/**
 * What the GPU status card says about the machine (design 8.1, queue
 * footer): the device, whether decoding and encoding run on the GPU, and
 * short capability chips.
 */
import type { Capabilities } from "@/bindings/Capabilities";
import type { EncoderInfo } from "@/bindings/EncoderInfo";

import { codecName } from "./media";

/** Overall state: GPU path, CPU fallback, or no working encoder. */
export type GpuHealth = "gpu" | "cpu-fallback" | "no-encoder";

/** The card's content once the probe has finished. */
export interface GpuSummary {
  readonly health: GpuHealth;
  /** Card title ("GPU ready", "CPU fallback", "No encoder"). */
  readonly title: string;
  /** Adapter name, or a fallback when DXGI reported none. */
  readonly device: string;
  /** Dedicated video memory ("16 GB"), when known. */
  readonly vram: string | null;
  /** Capability chips, for example "NVDEC", "NVENC H.264/HEVC/AV1", "OCR DirectML". */
  readonly chips: readonly string[];
}

/** Display names of the OCR execution providers. */
const OCR_PROVIDERS = { DirectMl: "DirectML", TensorRtRtx: "TensorRT", Cpu: "CPU" } as const;

/** Codec order in the chips. */
const CODEC_ORDER = ["h264", "hevc", "av1"];

/** The hardware method of an encoder name ("h264_nvenc" → "nvenc"). */
function method(encoder: EncoderInfo): string {
  const index = encoder.name.lastIndexOf("_");
  return index === -1 ? encoder.name : encoder.name.slice(index + 1);
}

/** "NVENC H.264/HEVC/AV1" chips, one per hardware method. */
function encoderChips(encoders: readonly EncoderInfo[]): string[] {
  const byMethod = new Map<string, Set<string>>();
  for (const encoder of encoders) {
    const codecs = byMethod.get(method(encoder)) ?? new Set<string>();
    codecs.add(encoder.codec);
    byMethod.set(method(encoder), codecs);
  }
  return [...byMethod].map(([name, codecs]) => {
    const ordered = [...codecs].toSorted((a, b) => rank(a) - rank(b)).map(codecName);
    return `${name.toUpperCase()} ${ordered.join("/")}`;
  });
}

function rank(codec: string): number {
  const index = CODEC_ORDER.indexOf(codec);
  return index === -1 ? CODEC_ORDER.length : index;
}

/** Binary gigabytes, rounded ("16 GB"). */
function formatVram(bytes: number): string | null {
  if (bytes <= 0) return null;
  return `${Math.round(bytes / 1024 ** 3)} GB`;
}

/** Summarises finished capabilities for the card. */
export function summarizeGpu(capabilities: Capabilities): GpuSummary {
  const gpu = capabilities.gpus[0];
  const hardwareDecoders = new Set(
    capabilities.decoders.filter((decoder) => decoder.hardware).map((decoder) => decoder.name),
  );
  const working = capabilities.encoders.filter((encoder) => encoder.available);
  const hardwareEncoders = working.filter((encoder) => encoder.hardware);

  const health: GpuHealth =
    hardwareEncoders.length > 0 ? "gpu" : working.length > 0 ? "cpu-fallback" : "no-encoder";
  const chips = [
    ...[...hardwareDecoders].map((name) => name.toUpperCase()),
    ...encoderChips(hardwareEncoders),
  ];
  if (health === "cpu-fallback") chips.push("CPU encoding");
  if (typeof capabilities.ocr === "object" && "Available" in capabilities.ocr) {
    chips.push(`OCR ${OCR_PROVIDERS[capabilities.ocr.Available.provider]}`);
  }

  return {
    health,
    title: { gpu: "GPU ready", "cpu-fallback": "CPU fallback", "no-encoder": "No encoder" }[health],
    device: gpu?.name ?? "No GPU found",
    vram: gpu ? formatVram(gpu.vram) : null,
    chips,
  };
}
