/**
 * Test entry point of the WebGL blur, installed only in the `mock` mode:
 * the end-to-end tests feed it the reference frames written by
 * `cargo run -p blurrd-blur --example reference_frames` and compare its
 * output with the frames `CpuBlur` produced.
 */
import type { BlurParams } from "@/bindings/BlurParams";
import type { RectI } from "@/bindings/RectI";
import { colourModel, pixelParams, prepareBoxes } from "@/lib/blur-geometry";

import { BlurRenderer } from "./blur-renderer";

/** One reference case, as described in `cases.json`. */
export interface BlurProbeCase {
  readonly width: number;
  readonly height: number;
  readonly sy: number;
  readonly params: BlurParams;
  readonly boxes: readonly {
    readonly rect: RectI;
    readonly seed: number;
    readonly grid_origin: readonly [number, number];
    readonly block_scale: number;
  }[];
}

/**
 * Blurs packed RGB `input` (base64) like the preview does and returns the
 * packed RGB result (base64). The reference frames are BT.709 limited
 * range.
 */
export function blurProbe(probe: BlurProbeCase, input: string): string {
  const { width, height } = probe;
  const rgb = Uint8Array.from(atob(input), (char) => char.charCodeAt(0));
  const rgba = new Uint8ClampedArray(width * height * 4);
  for (let i = 0; i < width * height; i += 1) {
    rgba.set(rgb.subarray(i * 3, i * 3 + 3), i * 4);
    rgba[i * 4 + 3] = 255;
  }
  const params = pixelParams(probe.params, probe.sy);
  if (!params) throw new Error("invalid blur parameters");
  const boxes = prepareBoxes(
    probe.boxes.map((b) => ({
      rect: b.rect,
      seed: b.seed,
      gridOrigin: b.grid_origin,
      blockScale: b.block_scale,
    })),
    width,
    height,
    params,
  );
  const renderer = new BlurRenderer(new OffscreenCanvas(width, height));
  try {
    renderer.upload(new ImageData(rgba, width, height), width, height);
    renderer.blur(boxes, params, colourModel("bt709", "Limited"));
    const out = renderer.readOutput();
    let binary = "";
    for (let i = 0; i < width * height; i += 1) {
      binary += String.fromCharCode(out[i * 4] ?? 0, out[i * 4 + 1] ?? 0, out[i * 4 + 2] ?? 0);
    }
    return btoa(binary);
  } finally {
    renderer.dispose();
  }
}
