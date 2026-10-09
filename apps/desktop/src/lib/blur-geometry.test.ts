import { describe, expect, it } from "vitest";

import type { BlurParams } from "@/bindings/BlurParams";
import type { RectI } from "@/bindings/RectI";

import {
  bilinearTaps,
  blockPx,
  boxGrid,
  cells,
  colourModel,
  gaussianKernel,
  noise,
  pixelParams,
  prepareBoxes,
  type BlurBox,
} from "./blur-geometry";

const DEFAULT: BlurParams = {
  style: "PixelBlur",
  strength: "Medium",
  pad: [4, 3],
  radius: 6,
  feather: 1.5,
  noise: true,
};

const BOXES: readonly BlurBox[] = [
  { rect: [101, 33, 177, 51], seed: 5, gridOrigin: [104, 35], blockScale: 1.5 },
  { rect: [0, 0, 9, 5], seed: 6, gridOrigin: [0, 0], blockScale: 1 },
  { rect: [630, 350, 640, 360], seed: 7, gridOrigin: [631, 351], blockScale: 0.5 },
];

/**
 * Expected values printed by `PixelParams::new` and `prepare_boxes` of
 * `blurrd-blur` for the same input (a 639 × 360 frame).
 */
const GOLDEN: readonly {
  readonly params: BlurParams;
  readonly sy: number;
  readonly block: number;
  readonly pad: readonly [number, number];
  readonly feather: number;
  readonly radius: number;
  readonly padded: readonly RectI[];
  readonly blocks: readonly number[];
}[] = [
  {
    params: DEFAULT,
    sy: 1,
    block: 10,
    pad: [4, 3],
    feather: 1.5,
    radius: 6,
    padded: [
      [96, 30, 182, 54],
      [0, 0, 14, 8],
      [626, 346, 639, 360],
    ],
    blocks: [15, 10, 10],
  },
  {
    params: DEFAULT,
    sy: 2 / 3,
    block: 6.666667,
    pad: [3, 2],
    feather: 1,
    radius: 4,
    padded: [
      [98, 30, 180, 54],
      [0, 0, 12, 8],
      [626, 348, 639, 360],
    ],
    blocks: [10, 6.666667, 6.666667],
  },
  {
    params: { ...DEFAULT, strength: "High" },
    sy: 2,
    block: 28,
    pad: [8, 6],
    feather: 3,
    radius: 12,
    padded: [
      [92, 26, 186, 58],
      [0, 0, 18, 12],
      [622, 344, 639, 360],
    ],
    blocks: [42, 28, 28],
  },
  {
    params: { ...DEFAULT, pad: [0, 0], feather: 4, radius: 20 },
    sy: 1,
    block: 10,
    pad: [0, 0],
    feather: 0.5,
    radius: 0.5,
    padded: [
      [100, 32, 178, 52],
      [0, 0, 10, 6],
      [630, 350, 639, 360],
    ],
    blocks: [15, 10, 10],
  },
  {
    params: { ...DEFAULT, strength: { Custom: 3 }, feather: 2.5, radius: 9, pad: [5, 1] },
    sy: 1.3333333,
    block: 10.666666,
    pad: [7, 2],
    feather: 2.5,
    radius: 2.5000002,
    padded: [
      [94, 30, 184, 54],
      [0, 0, 16, 8],
      [622, 348, 639, 360],
    ],
    blocks: [15.999999, 10.666666, 10.666666],
  },
  {
    params: DEFAULT,
    sy: 0.25,
    block: 2.5,
    pad: [1, 1],
    feather: 0.375,
    radius: 1.5,
    padded: [
      [100, 32, 178, 52],
      [0, 0, 10, 6],
      [628, 348, 639, 360],
    ],
    blocks: [3.75, 2.5, 2.5],
  },
];

describe("pixelParams and prepareBoxes", () => {
  it.each(GOLDEN)("match blurrd-blur at sy $sy", (golden) => {
    const params = pixelParams(golden.params, golden.sy);
    expect(params).not.toBeNull();
    if (!params) return;
    expect(params.block).toBeCloseTo(golden.block, 5);
    expect(params.pad).toEqual(golden.pad);
    expect(params.feather).toBeCloseTo(golden.feather, 6);
    expect(params.radius).toBeCloseTo(golden.radius, 6);
    const prepared = prepareBoxes(BOXES, 639, 360, params);
    expect(prepared.map((b) => b.padded)).toEqual(golden.padded);
    expect(prepared.map((b) => b.core)).toEqual([
      [101, 33, 177, 51],
      [0, 0, 9, 5],
      [630, 350, 639, 360],
    ]);
    prepared.forEach((b, i) => expect(b.block).toBeCloseTo(golden.blocks[i] ?? 0, 5));
  });

  it("rejects scales and feathers the engine rejects", () => {
    expect(pixelParams(DEFAULT, 0.1)).toBeNull();
    expect(pixelParams(DEFAULT, Number.NaN)).toBeNull();
    expect(pixelParams({ ...DEFAULT, feather: -1 }, 1)).toBeNull();
  });

  it("drops boxes outside the frame", () => {
    const params = pixelParams(DEFAULT, 1);
    if (!params) throw new Error("params");
    const outside: BlurBox = {
      rect: [700, 10, 720, 20],
      seed: 1,
      gridOrigin: [700, 10],
      blockScale: 1,
    };
    expect(prepareBoxes([outside], 639, 360, params)).toEqual([]);
  });

  it("maps strengths to block sizes with the floor", () => {
    expect(["Low", "Medium", "High"].map((s) => blockPx(s as "Low"))).toEqual([8, 10, 14]);
    expect(blockPx({ Custom: 3 })).toBe(8);
    expect(blockPx({ Custom: 20 })).toBe(20);
  });
});

describe("block grid", () => {
  it("cuts cells at the area and keeps uncut centres", () => {
    const grid = cells(3, 10, 0, 30);
    expect(grid.bounds).toEqual([0, 3, 13, 23, 30]);
    expect(grid.centres).toEqual([-2, 8, 18, 28]);
    expect(grid.first).toBe(-1);
  });

  it("replicates the border in the bilinear taps", () => {
    const taps = bilinearTaps({ bounds: [0, 10, 20], centres: [5, 15], first: 0 }, 20);
    expect(taps[0]).toEqual([0, 0, 0]);
    expect(taps[10]).toEqual([0, 1, Math.fround(0.55)]);
    expect(taps[19]).toEqual([1, 1, 0]);
  });

  it("builds the chroma grid at half resolution", () => {
    const params = pixelParams(DEFAULT, 1);
    if (!params) throw new Error("params");
    const [b] = prepareBoxes([BOXES[0] as BlurBox], 639, 360, params);
    if (!b) throw new Error("box");
    const luma = boxGrid(b, params.fill, 1);
    const chroma = boxGrid(b, params.fill, 2);
    expect([luma.width, luma.height]).toEqual([86, 24]);
    expect([chroma.width, chroma.height]).toEqual([43, 12]);
    expect(chroma.kernel.length).toBeLessThan(luma.kernel.length);
  });
});

describe("gaussianKernel", () => {
  it("is normalised with radius ceil(3 sigma)", () => {
    const kernel = gaussianKernel(3.5);
    expect(kernel.length).toBe(23);
    expect(kernel.reduce((a, b) => a + b, 0)).toBeCloseTo(1, 6);
    expect(kernel[11]).toBeGreaterThan(kernel[10] ?? 1);
  });
});

describe("noise", () => {
  it("matches the SplitMix64 reference for positive and negative block indices", () => {
    // Values from an independent Python implementation of the same hash.
    expect(noise(7, 0, 0)).toBe(-0.8540592193603516);
    expect(noise(4255298252, -1, 3)).toBe(0.3057297468185425);
    expect(noise(0, 12, -5)).toBe(-0.2044081687927246);
  });

  it("stays in -1..1", () => {
    for (let k = -20; k < 20; k += 1) {
      const value = noise(123, k, -k);
      expect(value).toBeGreaterThanOrEqual(-1);
      expect(value).toBeLessThan(1);
    }
  });
});

describe("colourModel", () => {
  it("picks the matrix weights and the noise scale per range", () => {
    expect(colourModel("bt709", "Limited")).toEqual({ luma: [0.2126, 0.0722], noise: 2 / 219 });
    expect(colourModel("smpte170m", "Full")).toEqual({ luma: [0.299, 0.114], noise: 2 / 255 });
    expect(colourModel("bt2020nc", "Limited").luma).toEqual([0.2627, 0.0593]);
    expect(colourModel("unknown", "Unspecified").luma).toEqual([0.2126, 0.0722]);
  });
});
