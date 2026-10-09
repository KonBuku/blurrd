/**
 * The blur geometry of `blurrd-blur`, ported for the WebGL preview: the
 * project's blur parameters resolved to source pixels (`PixelParams`), the
 * boxes padded, aligned and clamped (`prepare_boxes`), the soft mask, the
 * block grid, the bilinear taps, the gaussian kernel and the per-block
 * noise. The CPU backend computes in 32-bit floats, so every float step
 * here is rounded with `Math.fround` in the same order; the WebGL canvas
 * then draws the same boxes the export blurs (plan C7).
 */
import type { BlurParams } from "@/bindings/BlurParams";
import type { ColorRange } from "@/bindings/ColorRange";
import type { RectI } from "@/bindings/RectI";
import type { Strength } from "@/bindings/Strength";

const f32 = Math.fround;

/** Smallest block size in reference pixels (`MIN_BLOCK_PX`). */
export const MIN_BLOCK_PX = 8;
/** Gaussian sigma of the pixel blur, per block size. */
export const PIXEL_BLUR_SIGMA_PER_BLOCK = f32(0.35);
/** Gaussian sigma of the gaussian style, per block size. */
export const GAUSSIAN_SIGMA_PER_BLOCK = f32(0.6);
/** Noise amplitude in 8-bit luma code values. */
export const NOISE_AMPLITUDE = 2;
/**
 * How the video encodes colour, as far as the preview blur needs it: the
 * luma weights of its matrix (to split a change into luma and chroma like
 * the 4:2:0 export) and the noise amplitude as a fraction of the RGB
 * range. The CPU backend adds the noise to luma code values; one luma code
 * value is 255/219 RGB code values in limited range and one in full range.
 */
export interface ColourModel {
  /** `[Kr, Kb]` of the matrix. */
  readonly luma: readonly [number, number];
  readonly noise: number;
}

/** The colour model of a stream's tags (BT.709 when unknown). */
export function colourModel(matrix: string, range: ColorRange): ColourModel {
  const luma: readonly [number, number] = ["bt470bg", "smpte170m", "bt601", "fcc"].includes(matrix)
    ? [0.299, 0.114]
    : matrix.startsWith("bt2020")
      ? [0.2627, 0.0593]
      : [0.2126, 0.0722];
  return { luma, noise: NOISE_AMPLITUDE / (range === "Full" ? 255 : 219) };
}

/** Supported HUD scales (`sy`). */
export const MIN_SCALE = 0.25;
export const MAX_SCALE = 8;

const SQRT_2 = f32(Math.SQRT2);

/** How a box is filled (the CPU backend's `Fill`). */
export type Fill =
  | { readonly kind: "smooth"; readonly sigmaPerBlock: number }
  | { readonly kind: "blocks" }
  | { readonly kind: "boxMean" }
  /** A fixed sRGB colour, 0–255 per channel. */
  | { readonly kind: "colour"; readonly rgb: readonly [number, number, number] };

/** Blur parameters resolved for one video, sizes in source pixels. */
export interface PixelParams {
  readonly fill: Fill;
  readonly block: number;
  readonly pad: readonly [number, number];
  readonly radius: number;
  readonly feather: number;
  readonly noise: boolean;
}

/** One box of a frame before preparation (`BlurBox`). */
export interface BlurBox {
  /** The unpadded box in source pixels. */
  readonly rect: RectI;
  /** Noise seed, constant over a track. */
  readonly seed: number;
  /** Origin of the block grid in source pixels. */
  readonly gridOrigin: readonly [number, number];
  /** Block size multiplier (1.0 or 1.5). */
  readonly blockScale: number;
}

/** A box resolved for one frame size (`PreparedBox`). */
export interface PreparedBox {
  /** Pixels the blur reads and writes: `[x0, y0, x1, y1]`, half-open. */
  readonly padded: RectI;
  /** The unpadded box clamped to the frame; always fully covered. */
  readonly core: RectI;
  /** Edges of the padded rounded rectangle before alignment. */
  readonly shape: RectI;
  readonly radius: number;
  readonly feather: number;
  readonly gridOrigin: readonly [number, number];
  /** Block size in source pixels, including the block scale. */
  readonly block: number;
  readonly seed: number;
}

/** The block size of a strength in reference pixels. */
export function blockPx(strength: Strength): number {
  switch (strength) {
    case "Low":
      return 8;
    case "Medium":
      return 10;
    case "High":
      return 14;
    default:
      return Math.max(strength.Custom, MIN_BLOCK_PX);
  }
}

/** The fill of a style. */
function fillOf(style: BlurParams["style"]): Fill {
  if (style === "Gaussian") return { kind: "smooth", sigmaPerBlock: GAUSSIAN_SIGMA_PER_BLOCK };
  if (style === "Pixelate") return { kind: "blocks" };
  if (style === "PixelBlur") return { kind: "smooth", sigmaPerBlock: PIXEL_BLUR_SIGMA_PER_BLOCK };
  const { color } = style.Solid;
  return color === "BoxMean" ? { kind: "boxMean" } : { kind: "colour", rgb: color.Rgb };
}

/**
 * Resolves `params` for a video with HUD scale `sy`, like
 * `PixelParams::new`; `null` for a scale outside 0.25–8 or an invalid
 * feather (the backend rejects those projects).
 */
export function pixelParams(params: BlurParams, sy: number): PixelParams | null {
  const scale = f32(sy);
  if (!(scale >= MIN_SCALE && scale <= MAX_SCALE)) return null;
  if (!Number.isFinite(params.feather) || params.feather < 0) return null;
  const block = f32(blockPx(params.strength) * scale);
  const pad = [
    Math.ceil(f32(params.pad[0] * scale)),
    Math.ceil(f32(params.pad[1] * scale)),
  ] as const;
  const inset = f32(Math.min(pad[0], pad[1]) + 0.5);
  const feather = Math.min(f32(f32(params.feather) * scale), inset);
  // `SQRT_2.mul_add(inset, -feather)`: the product of two f32 values is
  // exact in f64, so one rounding matches the fused operation.
  const maxRadius = f32(f32(SQRT_2 * inset - feather) / f32(SQRT_2 - 1));
  const radius = Math.max(Math.min(f32(params.radius * scale), maxRadius), 0);
  return { fill: fillOf(params.style), block, pad, radius, feather, noise: params.noise };
}

/**
 * Pads, aligns (outward to even coordinates) and clamps `boxes` for a
 * `width` × `height` frame, like `prepare_boxes`. Boxes outside the frame
 * are dropped.
 */
export function prepareBoxes(
  boxes: readonly BlurBox[],
  width: number,
  height: number,
  params: PixelParams,
): PreparedBox[] {
  const [padX, padY] = params.pad;
  const out: PreparedBox[] = [];
  for (const b of boxes) {
    const core = intersect(b.rect, [0, 0, width, height]);
    if (!core) continue;
    const [x0, y0, x1, y1] = b.rect;
    const shape: RectI = [x0 - padX, y0 - padY, x1 + padX, y1 + padY];
    const aligned: RectI = [shape[0] & ~1, shape[1] & ~1, (shape[2] + 1) & ~1, (shape[3] + 1) & ~1];
    const padded = intersect(aligned, [0, 0, width, height]);
    if (!padded) continue;
    const halfMin = f32(Math.min(shape[2] - shape[0], shape[3] - shape[1]) * 0.5);
    const scale = Number.isFinite(b.blockScale) ? Math.max(f32(b.blockScale), 1) : 1;
    out.push({
      padded,
      core,
      shape,
      radius: Math.min(params.radius, halfMin),
      feather: params.feather,
      gridOrigin: b.gridOrigin,
      block: f32(params.block * scale),
      seed: b.seed,
    });
  }
  return out;
}

/** The overlap of two half-open rectangles, or `null` without area. */
export function intersect(a: RectI, b: RectI): RectI | null {
  const r: RectI = [
    Math.max(a[0], b[0]),
    Math.max(a[1], b[1]),
    Math.min(a[2], b[2]),
    Math.min(a[3], b[3]),
  ];
  return r[0] < r[2] && r[1] < r[3] ? r : null;
}

/** Cells of the block grid along one axis, cut to `lo..hi`. */
export interface Cells {
  /** Cell boundaries; cell `i` is `bounds[i]..bounds[i + 1]`. */
  readonly bounds: readonly number[];
  /** Centres of the uncut cells. */
  readonly centres: readonly number[];
  /** Grid index of the first cell. */
  readonly first: number;
}

/** Cells of size `block` with an edge at `origin` overlapping `lo..hi`. */
export function cells(origin: number, block: number, lo: number, hi: number): Cells {
  const edge = (k: number) => Math.floor(origin + k * block + 0.5);
  let k = Math.floor((lo - origin) / block) - 1;
  while (edge(k + 1) <= lo) k += 1;
  const first = k;
  const bounds = [lo];
  const centres: number[] = [];
  while (edge(k) < hi) {
    const e0 = edge(k);
    const e1 = edge(k + 1);
    bounds.push(Math.min(e1, hi));
    centres.push((e0 + e1) * 0.5);
    k += 1;
  }
  return { bounds, centres, first };
}

/** A bilinear tap: the two cells and the weight of the second. */
export type Tap = readonly [number, number, number];

/**
 * The bilinear taps of `len` samples between the cell centres; samples
 * outside the first or last centre take that cell (replicated border).
 */
export function bilinearTaps(grid: Cells, len: number): Tap[] {
  const c = grid.centres;
  const last = c.length - 1;
  const taps: Tap[] = [];
  let j = 0;
  for (let i = 0; i < len; i += 1) {
    const p = i + 0.5;
    while (j < last && (c[j + 1] ?? Infinity) <= p) j += 1;
    const cj = c[j] ?? 0;
    if (p <= (c[0] ?? 0)) taps.push([0, 0, 0]);
    else if (j === last) taps.push([last, last, 0]);
    else taps.push([j, j + 1, f32(f32(p - cj) / f32((c[j + 1] ?? cj) - cj))]);
  }
  return taps;
}

/** Normalised gaussian weights for offsets `-r..=r`, `r = ceil(3 sigma)`. */
export function gaussianKernel(sigma: number): Float32Array {
  const s = f32(sigma);
  const radius = Math.max(Math.ceil(f32(3 * s)), 1);
  const weights = new Float32Array(2 * radius + 1);
  const denominator = f32(f32(2 * s) * s);
  let sum = 0;
  for (let t = -radius; t <= radius; t += 1) {
    const w = f32(Math.exp(f32(-(t * t) / denominator)));
    weights[t + radius] = w;
    sum = f32(sum + w);
  }
  for (let i = 0; i < weights.length; i += 1) weights[i] = f32((weights[i] ?? 0) / sum);
  return weights;
}

const MASK64 = (1n << 64n) - 1n;

/** `v` as an unsigned 64-bit value (two's complement for negatives). */
function u64(v: bigint): bigint {
  return v & MASK64;
}

/**
 * Deterministic noise in -1..1 for block `(kx, ky)` of a box with `seed`
 * (SplitMix64 finaliser over the seed and the block index).
 */
export function noise(seed: number, kx: number, ky: number): number {
  let z =
    BigInt(seed >>> 0) ^
    u64(BigInt(kx) * 0x9e3779b97f4a7c15n) ^
    u64(BigInt(ky) * 0xc2b2ae3d27d4eb4fn);
  z = u64((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n);
  z = u64((z ^ (z >> 27n)) * 0x94d049bb133111ebn);
  z ^= z >> 31n;
  return Number(z >> 40n) / (1 << 23) - 1;
}

/** The cell of every sample `0..len` of a grid. */
function cellOf(grid: Cells, len: number): Int32Array {
  const out = new Int32Array(len);
  for (let c = 0; c + 1 < grid.bounds.length; c += 1) {
    out.fill(c, grid.bounds[c], grid.bounds[c + 1]);
  }
  return out;
}

/** A box's block grid and taps at one resolution, relative to its padded box. */
export interface BoxGrid {
  /** Size of the padded box at this resolution. */
  readonly width: number;
  readonly height: number;
  readonly cols: Cells;
  readonly rows: Cells;
  /** Horizontal taps per column. */
  readonly colTaps: readonly Tap[];
  /** Vertical taps per row. */
  readonly rowTaps: readonly Tap[];
  /** Cell of every column and row (for hard blocks). */
  readonly colCell: Int32Array;
  readonly rowCell: Int32Array;
  /** Gaussian weights of the smooth fill at this resolution, else `[1]`. */
  readonly kernel: Float32Array;
}

/**
 * The block grid of a prepared box at luma (`subsampling` 1) or 4:2:0
 * chroma (2) resolution, like the CPU backend's `Geometry::build`: the
 * padded box halved with the far edges rounded up, the grid origin and
 * the block size halved, the gaussian sigma in chroma samples.
 */
export function boxGrid(b: PreparedBox, fill: Fill, subsampling: 1 | 2): BoxGrid {
  const s = subsampling;
  const [px0, py0, px1, py1] = b.padded;
  const x0 = Math.floor(px0 / s);
  const y0 = Math.floor(py0 / s);
  const width = Math.ceil(px1 / s) - x0;
  const height = Math.ceil(py1 / s) - y0;
  const block = f32(b.block / s);
  const local = (g: number, start: number) => f32((g - s * start) / s);
  const whole = fill.kind === "boxMean" || fill.kind === "colour";
  const cols = whole
    ? { bounds: [0, width], centres: [width / 2], first: 0 }
    : cells(local(b.gridOrigin[0], x0), block, 0, width);
  const rows = whole
    ? { bounds: [0, height], centres: [height / 2], first: 0 }
    : cells(local(b.gridOrigin[1], y0), block, 0, height);
  return {
    width,
    height,
    cols,
    rows,
    colTaps: bilinearTaps(cols, width),
    rowTaps: bilinearTaps(rows, height),
    colCell: cellOf(cols, width),
    rowCell: cellOf(rows, height),
    kernel:
      fill.kind === "smooth"
        ? gaussianKernel(f32(f32(fill.sigmaPerBlock * b.block) / s))
        : new Float32Array([1]),
  };
}
