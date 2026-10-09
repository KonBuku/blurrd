/**
 * The editing actions of the preview: what the manual box tool, the
 * selection handles, the editing shortcuts and the edit buttons do. Each
 * action reads the shown project's edits, applies one operation of
 * `lib/edits.ts` and commits the result to the edit store as one undo
 * step (`OverrideSync` then saves it).
 */
import type { Override } from "@/bindings/Override";
import type { RectI } from "@/bindings/RectI";
import type { OverlayEdit } from "@/features/preview/overlay-model";
import type { LoadedProject } from "@/features/preview/useProject";
import {
  addBox,
  adjustTrackBox,
  defaultRange,
  dragRect,
  hideTrackBox,
  newUid,
  removeEdit,
  setBoxRect,
  setRange,
  type EdgeMask,
} from "@/lib/edits";
import { useEditsStore } from "@/store/edits";
import { usePreviewStore } from "@/store/preview";

/** Commits `change(current list)` as one undo step (nothing when it returns the same list). */
function commitWith(change: (list: readonly Override[]) => readonly Override[]): void {
  const store = useEditsStore.getState();
  const next = change(store.overrides);
  if (next !== store.overrides) store.commit(next);
}

/**
 * Starts or stops the manual box tool; it starts with one second either
 * side of `frame`.
 */
export function toggleManualTool(frame: number, data: LoadedProject): void {
  const preview = usePreviewStore.getState();
  if (preview.tool) preview.endTool();
  else {
    const { clock } = data;
    preview.setTool(defaultRange(frame, 1 / clock.frameDuration, clock.count));
  }
}

/** Overlay key of an edit. */
export function editKey(uid: string): string {
  return `edit:${uid}`;
}

/** Adds a manual box over `range` and selects it. */
export function addManualBox(range: readonly [number, number], rect: RectI): void {
  const uid = newUid();
  commitWith((list) => addBox(list, uid, range, rect));
  usePreviewStore.getState().select(editKey(uid));
}

/** Moves or resizes the box of an editable item to `rect`. */
export function setItemRect(edit: OverlayEdit, rect: RectI): void {
  if (edit.kind === "manual") commitWith((list) => setBoxRect(list, edit.uid, rect));
  else if (edit.kind === "track")
    commitWith((list) => adjustTrackBox(list, edit.target, rect, newUid()));
}

/**
 * Deletes what an item stands for: a manual box or another edit is
 * removed, a detector box is hidden. Clears the selection.
 */
export function deleteItem(edit: OverlayEdit): void {
  if (edit.kind === "track") commitWith((list) => hideTrackBox(list, edit.target, newUid()));
  else commitWith((list) => removeEdit(list, edit.uid));
  usePreviewStore.getState().select(null);
}

/** Sets the frames of a manual box. */
export function setManualRange(uid: string, range: readonly [number, number]): void {
  commitWith((list) => setRange(list, uid, range));
}

/** Moves (`edges` = all) or resizes an item's box by whole source pixels, inside the frame. */
export function nudgeItem(
  edit: OverlayEdit,
  rect: RectI,
  edges: EdgeMask,
  dx: number,
  dy: number,
  data: LoadedProject,
): void {
  const { width, height } = data.project.media;
  setItemRect(edit, dragRect(rect, edges, dx, dy, width, height));
}

/**
 * The editable thing the selection key `key` stands for on `frame`, with
 * the box shown there: a manual box, a detector box (its track's box on
 * that frame) or another edit; `null` for leaks, unknown keys or a track
 * without a box on that frame.
 */
export function resolveSelection(
  key: string | null,
  frame: number,
  data: LoadedProject,
): { readonly edit: OverlayEdit; readonly rect: RectI } | null {
  if (key === null) return null;
  if (key.startsWith("edit:")) {
    const uid = key.slice("edit:".length);
    const edit = data.tracks.overrides.find((candidate) => candidate.uid === uid);
    if (!edit) return null;
    if (edit.op === "Add") {
      return { edit: { kind: "manual", uid, range: [edit.f0, edit.f1] }, rect: edit.rect };
    }
    return { edit: { kind: "edit", uid }, rect: edit.anchor };
  }
  if (key.startsWith("track:")) {
    const id = Number(key.slice("track:".length));
    const box = data.index.boxesAt(frame, data.enabled).find((b) => b.trackId === id);
    if (!box) return null;
    return {
      edit: {
        kind: "track",
        target: {
          region: box.region,
          span: box.span,
          source: box.source,
          adjustUid: box.adjustUid,
        },
      },
      rect: box.rect,
    };
  }
  return null;
}
