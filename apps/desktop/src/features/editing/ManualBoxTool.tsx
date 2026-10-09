import { useState, type PointerEvent as ReactPointerEvent } from "react";

import type { RectI } from "@/bindings/RectI";
import type { LoadedProject } from "@/features/preview/useProject";
import { formatTimecode } from "@/lib/frames";
import { formatCount } from "@/lib/media";
import { rectFromCorners } from "@/lib/edits";
import { toSource, type StageBox } from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";

import { addManualBox } from "./actions";

export interface ManualBoxToolProps {
  readonly data: LoadedProject;
  /** Where the frame is on the stage. */
  readonly box: StageBox;
  /** The frames the new box covers (first and last, inclusive). */
  readonly range: readonly [number, number];
}

/**
 * The manual box tool (design 8.1, M): a crosshair layer over the stage;
 * dragging a rectangle adds a Manual box over the tool's frames (one
 * second either side of the frame where the tool started, or the in and
 * out points set with I and O), selects it and leaves the tool. A hint
 * at the top says the frames and the keys. The box snaps to source
 * pixels and is clipped to the frame.
 */
export function ManualBoxTool({ data, box, range }: ManualBoxToolProps) {
  const [draft, setDraft] = useState<RectI | null>(null);
  const { width, height } = data.project.media;
  const { clock } = data;

  const onPointerDown = (event: ReactPointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    const element = event.currentTarget;
    element.setPointerCapture(event.pointerId);
    const bounds = element.getBoundingClientRect();
    const at = (e: { clientX: number; clientY: number }) =>
      toSource(box, { x: e.clientX - bounds.left, y: e.clientY - bounds.top });
    const start = at(event);
    let latest: RectI | null = null;
    const move = (e: PointerEvent) => {
      latest = rectFromCorners(start, at(e), width, height);
      setDraft(latest);
    };
    const finish = (commit: boolean) => () => {
      element.removeEventListener("pointermove", move);
      element.removeEventListener("pointerup", done);
      element.removeEventListener("pointercancel", cancel);
      setDraft(null);
      if (commit && latest) {
        addManualBox(range, latest);
        usePreviewStore.getState().endTool();
      }
    };
    const done = finish(true);
    const cancel = finish(false);
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerup", done);
    element.addEventListener("pointercancel", cancel);
  };

  const [f0, f1] = range;
  return (
    <div
      data-no-pan
      data-testid="manual-box-tool"
      className="pointer-events-auto absolute inset-0 cursor-crosshair"
      onPointerDown={onPointerDown}
    >
      {draft && (
        <div
          aria-hidden
          data-testid="manual-box-draft"
          className="absolute border-2 border-dashed border-region-manual bg-region-manual-surface"
          style={{
            left: box.left + draft[0] * box.scale,
            top: box.top + draft[1] * box.scale,
            width: (draft[2] - draft[0]) * box.scale,
            height: (draft[3] - draft[1]) * box.scale,
          }}
        />
      )}
      <output className="pointer-events-none absolute top-2 left-1/2 max-w-[calc(100%-1rem)] -translate-x-1/2 truncate rounded-full bg-black/70 px-3 py-1 text-ui-sm text-white">
        Draw a box · {formatTimecode(clock.startOf(f0))}–{formatTimecode(clock.startOf(f1))}{" "}
        <span className="sr-only">
          (frames {formatCount(f0)} to {formatCount(f1)})
        </span>
        · I / O set in and out · Esc cancels
      </output>
    </div>
  );
}
