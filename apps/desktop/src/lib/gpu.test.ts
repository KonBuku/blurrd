import { describe, expect, it } from "vitest";

import type { Capabilities } from "@/bindings/Capabilities";
import { MOCK_CAPABILITIES } from "@/ipc/mock/capabilities";

import { summarizeGpu } from "./gpu";

/** The mock machine with changed fields. */
function machine(patch: Partial<Capabilities>): Capabilities {
  return { ...MOCK_CAPABILITIES, ...patch };
}

describe("summarizeGpu", () => {
  it("describes a machine with GPU decoding and encoding", () => {
    expect(summarizeGpu(MOCK_CAPABILITIES)).toEqual({
      health: "gpu",
      title: "GPU ready",
      device: "NVIDIA GeForce RTX 4080 SUPER",
      vram: "16 GB",
      chips: ["NVDEC", "NVENC H.264/HEVC/AV1", "OCR DirectML"],
    });
  });

  it("groups encoders per hardware method in codec order", () => {
    const summary = summarizeGpu(
      machine({
        decoders: [{ codec: "hevc", name: "d3d11va", hardware: true, ten_bit: true }],
        encoders: [
          { codec: "hevc", name: "hevc_amf", hardware: true, ten_bit: true, available: true },
          { codec: "h264", name: "h264_amf", hardware: true, ten_bit: false, available: true },
          { codec: "av1", name: "av1_amf", hardware: true, ten_bit: true, available: false },
        ],
        ocr: "NotProbed",
      }),
    );
    expect(summary.chips).toEqual(["D3D11VA", "AMF H.264/HEVC"]);
  });

  it("reports a CPU fallback when only software encoders work", () => {
    const summary = summarizeGpu(
      machine({
        encoders: [
          { codec: "h264", name: "h264_nvenc", hardware: true, ten_bit: false, available: false },
          { codec: "h264", name: "libx264", hardware: false, ten_bit: true, available: true },
        ],
        ocr: { Unavailable: { reason: "model missing" } },
      }),
    );
    expect(summary.health).toBe("cpu-fallback");
    expect(summary.title).toBe("CPU fallback");
    expect(summary.chips).toEqual(["NVDEC", "CPU encoding"]);
  });

  it("reports a machine without a working encoder or GPU", () => {
    const summary = summarizeGpu(machine({ gpus: [], decoders: [], encoders: [] }));
    expect(summary).toMatchObject({
      health: "no-encoder",
      title: "No encoder",
      device: "No GPU found",
      vram: null,
    });
  });
});
