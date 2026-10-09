/**
 * User edits of a plan (plan 5.1 `Override`, 6.1): pure operations on the
 * override list that the manual box tool, the selection handles and the
 * Delete key apply. Every operation returns a new list (or the same list
 * when nothing changes), so the edit store can keep each version for
 * undo and send the whole list to the backend.
 */
import type { Override } from "@/bindings/Override";
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";

/** Smallest box side the tools create, in source pixels. */
export const MIN_BOX_SIDE = 2;

/** A detector box the user acts on: where it is and the track it belongs to. */
export interface TrackBoxTarget {
  readonly region: Region;
  /** First and last frame of the track: the range of a new edit. */
  readonly span: readonly [number, number];
  /** The detector's box on the shown frame (the anchor of a new edit). */
  readonly source: RectI;
  /** The `Adjust` edit that already moves this box, if any. */
  readonly adjustUid: string | null;
}

/** A new random edit id (a version 4 UUID; the engine accepts any version). */
export function newUid(): string {
  return crypto.randomUUID();
}

/** Whether two rectangles are the same. */
export function sameRect(a: RectI, b: RectI): boolean {
  return a[0] === b[0] && a[1] === b[1] && a[2] === b[2] && a[3] === b[3];
}

/**
 * The rectangle spanned by two corner points (any order), rounded to
 * whole source pixels and clipped to the frame; `null` when it is smaller
 * than {@link MIN_BOX_SIDE} on either side.
 */
export function rectFromCorners(
  a: { readonly x: number; readonly y: number },
  b: { readonly x: number; readonly y: number },
  width: number,
  height: number,
): RectI | null {
  const clampX = (v: number) => Math.min(Math.max(Math.round(v), 0), width);
  const clampY = (v: number) => Math.min(Math.max(Math.round(v), 0), height);
  const x0 = clampX(Math.min(a.x, b.x));
  const x1 = clampX(Math.max(a.x, b.x));
  const y0 = clampY(Math.min(a.y, b.y));
  const y1 = clampY(Math.max(a.y, b.y));
  if (x1 - x0 < MIN_BOX_SIDE || y1 - y0 < MIN_BOX_SIDE) return null;
  return [x0, y0, x1, y1];
}

/** Which edges a drag moves: a resize handle, or all four for a move. */
export interface EdgeMask {
  readonly left: boolean;
  readonly top: boolean;
  readonly right: boolean;
  readonly bottom: boolean;
}

/** The mask of a move: every edge follows the pointer. */
export const MOVE: EdgeMask = { left: true, top: true, right: true, bottom: true };

/**
 * `rect` with the edges in `edges` moved by `(dx, dy)` source pixels
 * (rounded). A move keeps the size and stays inside the frame; a resize
 * stops each edge {@link MIN_BOX_SIDE} pixels before the opposite one and
 * at the frame border.
 */
export function dragRect(
  rect: RectI,
  edges: EdgeMask,
  dx: number,
  dy: number,
  width: number,
  height: number,
): RectI {
  const [x0, y0, x1, y1] = rect;
  const mx = Math.round(dx);
  const my = Math.round(dy);
  if (edges.left && edges.right && edges.top && edges.bottom) {
    const sx = Math.min(Math.max(mx, -x0), width - x1);
    const sy = Math.min(Math.max(my, -y0), height - y1);
    return [x0 + sx, y0 + sy, x1 + sx, y1 + sy];
  }
  return [
    edges.left ? Math.min(Math.max(x0 + mx, 0), x1 - MIN_BOX_SIDE) : x0,
    edges.top ? Math.min(Math.max(y0 + my, 0), y1 - MIN_BOX_SIDE) : y0,
    edges.right ? Math.max(Math.min(x1 + mx, width), x0 + MIN_BOX_SIDE) : x1,
    edges.bottom ? Math.max(Math.min(y1 + my, height), y0 + MIN_BOX_SIDE) : y1,
  ];
}

/** The frames a new manual box covers by default: one second either side, clipped. */
export function defaultRange(frame: number, fps: number, count: number): [number, number] {
  const second = Math.max(1, Math.round(fps));
  return [Math.max(0, frame - second), Math.min(count - 1, frame + second)];
}

/** Sets the in point of a range, moving the out point along when it would end before it. */
export function withIn(range: readonly [number, number], frame: number): [number, number] {
  return [frame, Math.max(frame, range[1])];
}

/** Sets the out point of a range, moving the in point along when it would start after it. */
export function withOut(range: readonly [number, number], frame: number): [number, number] {
  return [Math.min(frame, range[0]), frame];
}

/** Adds a manual box at the end of the list. */
export function addBox(
  list: readonly Override[],
  uid: string,
  range: readonly [number, number],
  rect: RectI,
): Override[] {
  return [...list, { op: "Add", uid, region: "Manual", f0: range[0], f1: range[1], rect }];
}

/** Replaces the edit `uid` with `change(edit)`; the same list when nothing changes. */
function replace(
  list: readonly Override[],
  uid: string,
  change: (edit: Override) => Override,
): readonly Override[] {
  const index = list.findIndex((edit) => edit.uid === uid);
  const edit = list[index];
  if (!edit) return list;
  const next = change(edit);
  if (next === edit) return list;
  return list.with(index, next);
}

/** Moves or resizes the box of an `Add` edit. */
export function setBoxRect(
  list: readonly Override[],
  uid: string,
  rect: RectI,
): readonly Override[] {
  return replace(list, uid, (edit) =>
    edit.op === "Add" && !sameRect(edit.rect, rect) ? { ...edit, rect } : edit,
  );
}

/** Changes the frames of an edit. */
export function setRange(
  list: readonly Override[],
  uid: string,
  range: readonly [number, number],
): readonly Override[] {
  return replace(list, uid, (edit) =>
    edit.f0 === range[0] && edit.f1 === range[1] ? edit : { ...edit, f0: range[0], f1: range[1] },
  );
}

/** Removes an edit (a hidden box shows again, a moved box goes back). */
export function removeEdit(list: readonly Override[], uid: string): readonly Override[] {
  const next = list.filter((edit) => edit.uid !== uid);
  return next.length === list.length ? list : next;
}

/**
 * Moves or resizes a detector box. An existing `Adjust` of the box is
 * changed (and removed when the box is back where the detector put it);
 * otherwise a new `Adjust` over the whole track is appended, anchored at
 * the detector's box, so it follows the track wherever the box is still
 * at that place (and rebinds after a re-analysis).
 */
export function adjustTrackBox(
  list: readonly Override[],
  target: TrackBoxTarget,
  rect: RectI,
  uid: string,
): readonly Override[] {
  if (target.adjustUid !== null) {
    const existing = list.find((edit) => edit.uid === target.adjustUid);
    if (existing?.op === "Adjust" && sameRect(existing.anchor, rect)) {
      return removeEdit(list, existing.uid);
    }
    return replace(list, target.adjustUid, (edit) =>
      edit.op === "Adjust" && !sameRect(edit.rect, rect) ? { ...edit, rect } : edit,
    );
  }
  if (sameRect(target.source, rect)) return list;
  return [
    ...list,
    {
      op: "Adjust",
      uid,
      region: target.region,
      f0: target.span[0],
      f1: target.span[1],
      anchor: target.source,
      rect,
    },
  ];
}

/** Hides a detector box over its whole track (a `Suppress` at its place). */
export function hideTrackBox(
  list: readonly Override[],
  target: TrackBoxTarget,
  uid: string,
): Override[] {
  return [
    ...list,
    {
      op: "Suppress",
      uid,
      region: target.region,
      f0: target.span[0],
      f1: target.span[1],
      anchor: target.source,
    },
  ];
}
