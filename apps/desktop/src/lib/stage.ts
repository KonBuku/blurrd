/**
 * Geometry of the video stage: where the source frame sits in the stage
 * at a zoom level and pan position. The canvas, the video element and the
 * box overlay all use the same box, so source pixel `(x, y)` is at stage
 * point `(left + x · scale, top + y · scale)` for all three.
 */

/** A width and a height in pixels. */
export interface Size {
  readonly width: number;
  readonly height: number;
}

/** A point; source pixels or stage pixels depending on the use. */
export interface Point {
  readonly x: number;
  readonly y: number;
}

/** Zoom of the stage: fit the frame into the stage, or a fixed scale (1 = 100 %). */
export type Zoom = "fit" | number;

/** Zoom levels of the zoom menu and the zoom shortcuts (design 8.1). */
export const ZOOM_LEVELS = [0.25, 0.5, 1, 2, 4] as const;

/** Limits of free zoom (Ctrl+wheel). */
export const MIN_ZOOM = 0.1;
export const MAX_ZOOM = 8;

/** Where the frame is drawn in the stage, in stage pixels. */
export interface StageBox {
  /** Stage pixels per source pixel. */
  readonly scale: number;
  readonly left: number;
  readonly top: number;
  readonly width: number;
  readonly height: number;
}

/** The scale at which `source` just fits into `stage` (letterboxed). */
export function fitScale(stage: Size, source: Size): number {
  if (source.width <= 0 || source.height <= 0) return 1;
  return Math.min(stage.width / source.width, stage.height / source.height);
}

/** The scale of a zoom setting. */
export function scaleOf(zoom: Zoom, stage: Size, source: Size): number {
  return zoom === "fit" ? fitScale(stage, source) : zoom;
}

/**
 * Keeps the source point shown at the stage centre inside the range where
 * the frame still covers the stage (or centres the frame along an axis
 * where it is smaller than the stage).
 */
export function clampCenter(center: Point, stage: Size, source: Size, scale: number): Point {
  if (!(scale > 0)) return { x: source.width / 2, y: source.height / 2 };
  const axis = (c: number, stageLength: number, sourceLength: number) => {
    const half = stageLength / (2 * scale);
    if (half * 2 >= sourceLength) return sourceLength / 2;
    return Math.min(Math.max(c, half), sourceLength - half);
  };
  return {
    x: axis(center.x, stage.width, source.width),
    y: axis(center.y, stage.height, source.height),
  };
}

/**
 * The frame's box for a zoom and the source point at the stage centre
 * (`null` = the frame centre). Positions are rounded to whole stage pixels
 * so the canvas is not resampled at sub-pixel offsets.
 */
export function stageBox(stage: Size, source: Size, zoom: Zoom, center: Point | null): StageBox {
  const scale = scaleOf(zoom, stage, source);
  const c = clampCenter(
    center ?? { x: source.width / 2, y: source.height / 2 },
    stage,
    source,
    scale,
  );
  return {
    scale,
    // `+ 0` turns a rounded -0 into 0.
    left: Math.round(stage.width / 2 - c.x * scale) + 0,
    top: Math.round(stage.height / 2 - c.y * scale) + 0,
    width: source.width * scale,
    height: source.height * scale,
  };
}

/** The source point under stage point `p`. */
export function toSource(box: StageBox, p: Point): Point {
  return { x: (p.x - box.left) / box.scale, y: (p.y - box.top) / box.scale };
}

/**
 * The centre that keeps source point `anchor` under stage point `p` after
 * zooming to `scale` (Ctrl+wheel zooms around the pointer).
 */
export function centerForAnchor(anchor: Point, p: Point, stage: Size, scale: number): Point {
  return {
    x: anchor.x + (stage.width / 2 - p.x) / scale,
    y: anchor.y + (stage.height / 2 - p.y) / scale,
  };
}

/** The next zoom level above (`direction` 1) or below (-1) `scale`. */
export function stepZoom(scale: number, direction: 1 | -1): number {
  if (direction === 1) return ZOOM_LEVELS.find((level) => level > scale + 1e-6) ?? ZOOM_LEVELS[4];
  return ZOOM_LEVELS.findLast((level) => level < scale - 1e-6) ?? ZOOM_LEVELS[0];
}
