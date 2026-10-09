/**
 * Geometry of the timeline (design 8.1): the visible frame range, frame ↔
 * x mapping that snaps to whole frames, zoom around a point, panning and
 * the ruler's tick spacing.
 */

/** The visible frames `start..end` (end exclusive, fractional while zoomed). */
export interface TimelineView {
  readonly start: number;
  readonly end: number;
}

/** Fewest frames the timeline shows when zoomed in fully. */
export const MIN_VISIBLE_FRAMES = 30;

/** Smallest distance between two labelled ruler ticks, in pixels. */
const MIN_TICK_SPACING = 72;

/** Candidate tick steps in seconds (sub-second steps snap to frames). */
const STEPS_S = [0.1, 0.25, 0.5, 1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600];

/** The whole video. */
export function fullView(frameCount: number): TimelineView {
  return { start: 0, end: Math.max(frameCount, 1) };
}

/** x (pixels from the left edge) of the start of `frame`. */
export function frameToX(frame: number, view: TimelineView, width: number): number {
  return ((frame - view.start) / (view.end - view.start)) * width;
}

/** The frame under `x`, clamped to the video. */
export function xToFrame(x: number, view: TimelineView, width: number, frameCount: number): number {
  const frame = Math.floor(view.start + (x / Math.max(width, 1)) * (view.end - view.start));
  return Math.min(Math.max(frame, 0), Math.max(frameCount - 1, 0));
}

/** `view` limited to the video, keeping its length when possible. */
function clampView(start: number, length: number, frameCount: number): TimelineView {
  const total = Math.max(frameCount, 1);
  const span = Math.min(Math.max(length, Math.min(MIN_VISIBLE_FRAMES, total)), total);
  const s = Math.min(Math.max(start, 0), total - span);
  return { start: s, end: s + span };
}

/** Zooms by `factor` (> 1 zooms in) keeping `anchor` (a frame) in place. */
export function zoomView(
  view: TimelineView,
  anchor: number,
  factor: number,
  frameCount: number,
): TimelineView {
  const length = (view.end - view.start) / factor;
  const ratio = (anchor - view.start) / (view.end - view.start);
  return clampView(anchor - ratio * length, length, frameCount);
}

/** Moves the view by `frames`. */
export function panView(view: TimelineView, frames: number, frameCount: number): TimelineView {
  return clampView(view.start + frames, view.end - view.start, frameCount);
}

/** Moves the view the least so that `frame` is visible. */
export function revealFrame(view: TimelineView, frame: number, frameCount: number): TimelineView {
  if (frame >= view.start && frame < view.end) return view;
  const length = view.end - view.start;
  const start = frame < view.start ? frame : frame - length + 1;
  return clampView(start, length, frameCount);
}

/** The ruler's tick step in seconds for a view of `width` pixels. */
export function tickStep(view: TimelineView, width: number, fps: number): number {
  const secondsPerPixel = (view.end - view.start) / fps / Math.max(width, 1);
  const wanted = secondsPerPixel * MIN_TICK_SPACING;
  return STEPS_S.find((step) => step >= wanted) ?? STEPS_S[STEPS_S.length - 1] ?? 3600;
}

/** Ruler label of a time: `m:ss`, with tenths for sub-second steps. */
export function tickLabel(seconds: number, step: number): string {
  const whole = Math.floor(seconds + 1e-6);
  const m = Math.floor(whole / 60);
  const s = String(whole % 60).padStart(2, "0");
  if (step >= 1) return `${m}:${s}`;
  const tenths = Math.floor((seconds - whole) * 10 + 1e-6);
  return `${m}:${s}.${tenths}`;
}
