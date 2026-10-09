/**
 * The synthetic test video of the mock backend. Every frame is drawn by
 * {@link drawFrame} (pure code, no fonts or real footage): a gradient
 * background, HUD-like name plates with text-like strokes that appear,
 * slide and fade like kill-feed rows, a round banner and a chat line, an
 * alignment target, and the frame's own index as a row of black and white
 * squares. `scripts/make-fixture-media.ts` encodes the frames into the
 * committed fixture videos; the mock reads the same geometry to answer
 * `get_tracks`, and draws frame indices for its Exact patches.
 */
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";

/** Size, rate and length of the fixture video. */
export const FIXTURE = {
  width: 640,
  height: 360,
  fps: 60,
  frames: 240,
  /** WebM stores timestamps in milliseconds. */
  timeBase: [1, 1000] as const,
} as const;

/** Frames between two timeline thumbnails. */
export const THUMB_EVERY = 30;

/** URLs of the fixture videos, served by the Vite dev server. */
export const FIXTURE_URLS = {
  /** VP9 in WebM: plays in every Chromium. */
  video: "/e2e/fixtures/media/hud-640x360-vp9.webm",
  /** HEVC in MP4: needs the platform decoder (the OS extension on Windows). */
  hevc: "/e2e/fixtures/media/hud-640x360-hevc.mp4",
  /** The proxy of the HEVC source: VP9, half size (like the 960×540 proxy). */
  proxy: "/e2e/fixtures/media/hud-320x180-proxy.webm",
} as const;

/** The frame index code: `BITS` squares of `CELL` px from `(X, Y)`. */
export const INDEX_CODE = { x: 8, y: 8, cell: 12, bits: 10 } as const;

/** The rectangle the index code covers, with a 4 px dark border. */
export const INDEX_RECT: RectI = [
  INDEX_CODE.x - 4,
  INDEX_CODE.y - 4,
  INDEX_CODE.x + INDEX_CODE.cell * INDEX_CODE.bits + 4,
  INDEX_CODE.y + INDEX_CODE.cell + 4,
];

/** A name plate of the fixture: where its text is on which frames. */
export interface Plate {
  readonly region: Region;
  /** First and last frame (inclusive). */
  readonly f0: number;
  readonly f1: number;
  /** Text box at rest. */
  readonly rect: RectI;
  /** Frames over which the plate slides in from the right. */
  readonly slide: number;
  /** Bar colour around the text. */
  readonly bar: readonly [number, number, number];
}

/** The plates, in drawing order. */
export const PLATES: readonly Plate[] = [
  { region: "KillFeed", f0: 12, f1: 200, rect: [470, 40, 540, 52], slide: 0, bar: [150, 40, 40] },
  { region: "KillFeed", f0: 12, f1: 200, rect: [560, 40, 628, 52], slide: 0, bar: [150, 40, 40] },
  { region: "KillFeed", f0: 52, f1: 220, rect: [480, 62, 548, 74], slide: 0, bar: [40, 120, 90] },
  { region: "KillFeed", f0: 52, f1: 220, rect: [566, 62, 628, 74], slide: 0, bar: [40, 120, 90] },
  { region: "Banner", f0: 70, f1: 160, rect: [270, 150, 370, 168], slide: 12, bar: [90, 60, 140] },
  { region: "TextChat", f0: 100, f1: 239, rect: [14, 290, 70, 300], slide: 0, bar: [30, 30, 36] },
];

/** A fixed outline for overlay alignment checks (a Manual box in the mock). */
export const TARGET_RECT: RectI = [301, 241, 341, 263];

/** The text box of a plate on frame `f`, or `null` when it is not shown. */
export function plateRect(plate: Plate, f: number): RectI | null {
  if (f < plate.f0 || f > plate.f1) return null;
  const left = plate.slide > 0 ? Math.max(0, plate.f0 + plate.slide - f) * 8 : 0;
  const [x0, y0, x1, y1] = plate.rect;
  return [x0 + left, y0, x1 + left, y1];
}

/** A small integer hash for the stroke pattern. */
function hash(a: number, b: number): number {
  let z = Math.imul(a, 0x9e3779b1) ^ Math.imul(b, 0x85ebca77);
  z ^= z >>> 15;
  z = Math.imul(z, 0x2c1b3c6d);
  return (z ^ (z >>> 12)) >>> 0;
}

/** Draws frame `f` into `rgb` (packed RGB, `FIXTURE.width` × `FIXTURE.height`). */
export function drawFrame(rgb: Uint8Array, f: number): void {
  const { width, height } = FIXTURE;
  const put = (x: number, y: number, c: readonly [number, number, number]) => {
    if (x < 0 || y < 0 || x >= width || y >= height) return;
    const i = (y * width + x) * 3;
    rgb[i] = c[0];
    rgb[i + 1] = c[1];
    rgb[i + 2] = c[2];
  };
  const fill = (r: RectI, c: readonly [number, number, number]) => {
    for (let y = r[1]; y < r[3]; y += 1) for (let x = r[0]; x < r[2]; x += 1) put(x, y, c);
  };
  // Background: a slow diagonal gradient that drifts with the frame.
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const t = (x + y + f * 2) % 512;
      const v = 40 + (t < 256 ? t : 511 - t) / 6;
      const i = (y * width + x) * 3;
      rgb[i] = v * 0.7;
      rgb[i + 1] = v * 0.85;
      rgb[i + 2] = v;
    }
  }
  for (const [index, plate] of PLATES.entries()) {
    const rect = plateRect(plate, f);
    if (!rect) continue;
    const [x0, y0, x1, y1] = rect;
    fill([x0 - 6, y0 - 4, x1 + 6, y1 + 4], plate.bar);
    // Text-like strokes: columns of a few pixels, a pattern per plate.
    for (let x = x0; x < x1; x += 1) {
      for (let y = y0; y < y1; y += 1) {
        const glyph = Math.floor((x - x0) / 6);
        const on = hash(glyph * 31 + index, Math.floor((y - y0) / 3)) % 3 !== 0;
        if (on && (x - x0) % 6 < 4) put(x, y, [240, 240, 240]);
      }
    }
  }
  const [tx0, ty0, tx1, ty1] = TARGET_RECT;
  fill(TARGET_RECT, [255, 0, 255]);
  fill([tx0 + 1, ty0 + 1, tx1 - 1, ty1 - 1], [20, 20, 20]);
  drawIndex(rgb, width, f);
}

/**
 * Draws the index code of frame `f` (border included) into packed RGB of
 * `stride` pixels whose top-left pixel is `origin` of the frame. With
 * `inverted`, the code squares are drawn inverted (the mock's Exact
 * patches, so tests can tell a patch from the video under it).
 */
export function drawIndex(
  rgb: Uint8Array,
  stride: number,
  f: number,
  origin: readonly [number, number] = [0, 0],
  inverted = false,
): void {
  const [ox, oy] = origin;
  for (let y = INDEX_RECT[1]; y < INDEX_RECT[3]; y += 1) {
    for (let x = INDEX_RECT[0]; x < INDEX_RECT[2]; x += 1) {
      const bit = Math.floor((x - INDEX_CODE.x) / INDEX_CODE.cell);
      const inside =
        y >= INDEX_CODE.y &&
        y < INDEX_CODE.y + INDEX_CODE.cell &&
        bit >= 0 &&
        bit < INDEX_CODE.bits;
      const on = inside && (((f >> (INDEX_CODE.bits - 1 - bit)) & 1) === 1) !== inverted;
      const v = on ? 255 : 0;
      rgb.set([v, v, v], ((y - oy) * stride + (x - ox)) * 3);
    }
  }
}

/**
 * Reads the frame index from RGBA pixels of the source frame (`stride`
 * pixels per row): the centre of each code square, thresholded at 50 %.
 */
export function readIndex(rgba: ArrayLike<number>, stride: number): number {
  let value = 0;
  for (let bit = 0; bit < INDEX_CODE.bits; bit += 1) {
    const x = INDEX_CODE.x + bit * INDEX_CODE.cell + INDEX_CODE.cell / 2;
    const y = INDEX_CODE.y + INDEX_CODE.cell / 2;
    value = (value << 1) | ((rgba[(y * stride + x) * 4] ?? 0) > 127 ? 1 : 0);
  }
  return value;
}
