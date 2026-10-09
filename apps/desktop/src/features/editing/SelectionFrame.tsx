import { useState, type PointerEvent as ReactPointerEvent } from "react";

import type { RectI } from "@/bindings/RectI";
import type { OverlayEdit } from "@/features/preview/overlay-model";
import { dragRect, MOVE, sameRect, type EdgeMask } from "@/lib/edits";
import type { StageBox } from "@/lib/stage";
import { cn } from "@/lib/utils";

/** The eight resize handles: which edges each one moves, where it sits and its cursor. */
const HANDLES: readonly {
  readonly name: string;
  readonly edges: EdgeMask;
  readonly x: number;
  readonly y: number;
  readonly cursor: string;
}[] = [
  {
    name: "nw",
    edges: { left: true, top: true, right: false, bottom: false },
    x: 0,
    y: 0,
    cursor: "cursor-nwse-resize",
  },
  {
    name: "n",
    edges: { left: false, top: true, right: false, bottom: false },
    x: 0.5,
    y: 0,
    cursor: "cursor-ns-resize",
  },
  {
    name: "ne",
    edges: { left: false, top: true, right: true, bottom: false },
    x: 1,
    y: 0,
    cursor: "cursor-nesw-resize",
  },
  {
    name: "e",
    edges: { left: false, top: false, right: true, bottom: false },
    x: 1,
    y: 0.5,
    cursor: "cursor-ew-resize",
  },
  {
    name: "se",
    edges: { left: false, top: false, right: true, bottom: true },
    x: 1,
    y: 1,
    cursor: "cursor-nwse-resize",
  },
  {
    name: "s",
    edges: { left: false, top: false, right: false, bottom: true },
    x: 0.5,
    y: 1,
    cursor: "cursor-ns-resize",
  },
  {
    name: "sw",
    edges: { left: true, top: false, right: false, bottom: true },
    x: 0,
    y: 1,
    cursor: "cursor-nesw-resize",
  },
  {
    name: "w",
    edges: { left: true, top: false, right: false, bottom: false },
    x: 0,
    y: 0.5,
    cursor: "cursor-ew-resize",
  },
];

/** Side of a handle in stage pixels. */
const HANDLE = 8;

export interface SelectionFrameProps {
  /** The selected box as shown on the frame. */
  readonly rect: RectI;
  readonly edit: OverlayEdit;
  /** Where the frame is on the stage. */
  readonly box: StageBox;
  /** Source size, the limit of moves and resizes. */
  readonly width: number;
  readonly height: number;
  /** Called once per finished drag that changed the box. */
  readonly onCommit: (edit: OverlayEdit, rect: RectI) => void;
}

/**
 * The selection of an editable box (design 8.1): eight resize handles and
 * a move area over the box. A drag shows the new box dashed and commits
 * it on release, as one undo step; moves and resizes snap to source
 * pixels and stay inside the frame. Pointer-only: the keyboard moves and
 * resizes the selected box with Alt+arrows and Alt+Shift+arrows.
 */
export function SelectionFrame({ rect, edit, box, width, height, onCommit }: SelectionFrameProps) {
  const [draft, setDraft] = useState<RectI | null>(null);

  const startDrag = (edges: EdgeMask) => (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.stopPropagation();
    const element = event.currentTarget;
    element.setPointerCapture(event.pointerId);
    const from = { x: event.clientX, y: event.clientY };
    let latest = rect;
    const move = (e: PointerEvent) => {
      latest = dragRect(
        rect,
        edges,
        (e.clientX - from.x) / box.scale,
        (e.clientY - from.y) / box.scale,
        width,
        height,
      );
      setDraft(latest);
    };
    const finish = (commit: boolean) => () => {
      element.removeEventListener("pointermove", move);
      element.removeEventListener("pointerup", done);
      element.removeEventListener("pointercancel", cancel);
      setDraft(null);
      if (commit && !sameRect(latest, rect)) onCommit(edit, latest);
    };
    const done = finish(true);
    const cancel = finish(false);
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerup", done);
    element.addEventListener("pointercancel", cancel);
  };

  const place = ([x0, y0, x1, y1]: RectI) => ({
    left: box.left + x0 * box.scale,
    top: box.top + y0 * box.scale,
    width: (x1 - x0) * box.scale,
    height: (y1 - y0) * box.scale,
  });
  const frame = place(draft ?? rect);

  return (
    <>
      {draft && (
        <div
          aria-hidden
          data-testid="selection-draft"
          className="pointer-events-none absolute border-2 border-dashed border-accent-strong"
          style={frame}
        />
      )}
      <div
        aria-hidden
        data-no-pan
        data-testid="selection-frame"
        className="pointer-events-auto absolute cursor-move"
        style={frame}
        onPointerDown={startDrag(MOVE)}
      >
        {HANDLES.map((handle) => (
          <div
            key={handle.name}
            data-handle={handle.name}
            className={cn(
              "absolute rounded-[2px] border border-accent-strong bg-background",
              handle.cursor,
            )}
            style={{
              width: HANDLE,
              height: HANDLE,
              left: `calc(${handle.x * 100}% - ${HANDLE / 2}px)`,
              top: `calc(${handle.y * 100}% - ${HANDLE / 2}px)`,
            }}
            onPointerDown={startDrag(handle.edges)}
          />
        ))}
      </div>
    </>
  );
}
