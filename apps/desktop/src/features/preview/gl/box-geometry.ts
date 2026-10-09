/**
 * Packs the per-box data of the blur shaders into one RGBA32F texture
 * image (rows in `shaders.ts`) and keeps recent ones: kill-feed and banner
 * names hold the same shape for seconds, so most frames reuse every box's
 * data.
 */
import { boxGrid, noise, type BoxGrid, type Fill, type PreparedBox } from "@/lib/blur-geometry";

import { CHROMA_ROWS, GEOMETRY_ROWS, NOISE_ROW } from "./shaders";

/** The packed data of one box. */
export interface PackedBox {
  /** Luma grid. */
  readonly luma: GridSize;
  /** Chroma grid (half resolution). */
  readonly chroma: GridSize;
  /** Texture image: `texWidth` × `texHeight` RGBA floats. */
  readonly data: Float32Array;
  readonly texWidth: number;
  readonly texHeight: number;
}

/** Sizes of one resolution's grid. */
export interface GridSize {
  /** Padded box at this resolution. */
  readonly width: number;
  readonly height: number;
  /** Block columns and rows. */
  readonly cols: number;
  readonly rows: number;
  /** Gaussian radius in samples (0 without a smooth fill). */
  readonly radius: number;
}

/** Most packed boxes kept. */
const CAPACITY = 256;

/**
 * Everything the packed data depends on, relative to the padded box (the
 * grid terms of the CPU backend's shape key, plus seed and fill).
 */
function keyOf(b: PreparedBox, fill: Fill): string {
  const [x, y] = b.padded;
  return [
    b.padded[2] - x,
    b.padded[3] - y,
    b.gridOrigin[0] - x,
    b.gridOrigin[1] - y,
    b.block,
    b.seed,
    fill.kind,
    fill.kind === "smooth" ? fill.sigmaPerBlock : 0,
  ].join(",");
}

function sizeOf(grid: BoxGrid): GridSize {
  return {
    width: grid.width,
    height: grid.height,
    cols: grid.cols.centres.length,
    rows: grid.rows.centres.length,
    radius: (grid.kernel.length - 1) / 2,
  };
}

/** Builds the packed data of a box. */
function pack(b: PreparedBox, fill: Fill): PackedBox {
  const luma = boxGrid(b, fill, 1);
  const chroma = boxGrid(b, fill, 2);
  const lumaSize = sizeOf(luma);
  const texWidth = Math.max(
    ...[luma, chroma].flatMap((g) => [g.width, g.height, g.cols.bounds.length, g.kernel.length]),
  );
  const texHeight = NOISE_ROW + lumaSize.rows;
  const data = new Float32Array(texWidth * texHeight * 4);
  const set = (x: number, y: number, values: readonly number[]) =>
    data.set(values, (y * texWidth + x) * 4);
  const fillSet = (
    grid: BoxGrid,
    base: number,
    scale: number,
    limits: readonly [number, number],
  ) => {
    grid.colTaps.forEach(([c0, c1, t], x) =>
      set(x, base + GEOMETRY_ROWS.colTaps, [c0, c1, t, grid.colCell[x] ?? 0]),
    );
    grid.rowTaps.forEach(([r0, r1, t], y) =>
      set(y, base + GEOMETRY_ROWS.rowTaps, [r0, r1, t, grid.rowCell[y] ?? 0]),
    );
    // Bounds in luma pixels of the padded box: a chroma sample covers two.
    const bounds = (cells: readonly number[], row: number, limit: number) => {
      for (let c = 0; c + 1 < cells.length; c += 1) {
        const from = Math.min((cells[c] ?? 0) * scale, limit);
        const to = Math.min((cells[c + 1] ?? 0) * scale, limit);
        set(c, row, [from, to, 0, 0]);
      }
    };
    bounds(grid.cols.bounds, base + GEOMETRY_ROWS.colBounds, limits[0]);
    bounds(grid.rows.bounds, base + GEOMETRY_ROWS.rowBounds, limits[1]);
    grid.kernel.forEach((weight, t) => set(t, base + GEOMETRY_ROWS.kernel, [weight, 0, 0, 0]));
  };
  fillSet(luma, 0, 1, [luma.width, luma.height]);
  fillSet(chroma, CHROMA_ROWS, 2, [luma.width, luma.height]);
  // The block fills add noise to luma; the box mean and colours have none.
  if (fill.kind === "smooth" || fill.kind === "blocks") {
    for (let r = 0; r < lumaSize.rows; r += 1) {
      for (let c = 0; c < lumaSize.cols; c += 1) {
        const value = noise(b.seed, luma.cols.first + c, luma.rows.first + r);
        set(c, NOISE_ROW + r, [value, 0, 0, 0]);
      }
    }
  }
  return { luma: lumaSize, chroma: sizeOf(chroma), data, texWidth, texHeight };
}

/** A small store of packed boxes, emptied when it grows past its capacity. */
export class BoxGeometryCache {
  private readonly entries = new Map<string, PackedBox>();

  /** The packed data of `b` for `fill`, built on first use. */
  get(b: PreparedBox, fill: Fill): PackedBox {
    const key = keyOf(b, fill);
    let entry = this.entries.get(key);
    if (!entry) {
      if (this.entries.size >= CAPACITY) this.entries.clear();
      entry = pack(b, fill);
      this.entries.set(key, entry);
    }
    return entry;
  }
}
