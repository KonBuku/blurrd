/** The machine the mock backend pretends to be. */
import type { Capabilities } from "@/bindings/Capabilities";

/** An RTX 40 card with NVDEC, NVENC for all three codecs and DirectML OCR. */
export const MOCK_CAPABILITIES: Capabilities = {
  gpus: [
    {
      luid: "00000000000a1b2c",
      name: "NVIDIA GeForce RTX 4080 SUPER",
      vram: 16 * 1024 ** 3,
      driver: "32.0.15.9186",
    },
  ],
  decoders: [
    { codec: "h264", name: "nvdec", hardware: true, ten_bit: false },
    { codec: "hevc", name: "nvdec", hardware: true, ten_bit: true },
    { codec: "av1", name: "nvdec", hardware: true, ten_bit: true },
    { codec: "h264", name: "software", hardware: false, ten_bit: true },
  ],
  encoders: [
    { codec: "h264", name: "h264_nvenc", hardware: true, ten_bit: false, available: true },
    { codec: "hevc", name: "hevc_nvenc", hardware: true, ten_bit: true, available: true },
    { codec: "av1", name: "av1_nvenc", hardware: true, ten_bit: true, available: true },
    { codec: "h264", name: "libx264", hardware: false, ten_bit: true, available: true },
  ],
  probe_state: "Ready",
  ocr: { Available: { provider: "DirectMl" } },
  ffmpeg_version: "8.0",
  app_version: "0.1.0",
};
