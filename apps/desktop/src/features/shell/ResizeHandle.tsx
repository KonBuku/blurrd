import { useRef, useState, type KeyboardEvent, type PointerEvent } from "react";

import { RESIZE_STEP, clamp } from "@/lib/layout";
import { cn } from "@/lib/utils";

export interface ResizeHandleProps {
  /** Accessible name, for example "Resize queue". */
  readonly label: string;
  /** Id of the panel this handle resizes. */
  readonly controls: string;
  /** Current panel width. */
  readonly value: number;
  readonly min: number;
  readonly max: number;
  /**
   * Which side of the handle the panel is on: dragging away from the
   * panel makes it wider.
   */
  readonly panel: "left" | "right";
  /** Called with the new width while dragging and on key presses. */
  readonly onResize: (width: number) => void;
  /** Called on double-click: back to the default width. */
  readonly onReset: () => void;
  /** Called when a pointer drag starts and ends. */
  readonly onDraggingChange?: (dragging: boolean) => void;
}

/** Width at the start of a drag and where the pointer went down. */
interface DragOrigin {
  readonly x: number;
  readonly width: number;
}

/**
 * A 4 px vertical splitter between two frames (the gap doubles as the
 * handle). Hover and drag show a 1 px line with faded ends. It is a
 * focusable `separator`: arrow keys move it by 16 px, Home and End jump to
 * the limits, a double-click resets the panel.
 */
export function ResizeHandle({
  label,
  controls,
  value,
  min,
  max,
  panel,
  onResize,
  onReset,
  onDraggingChange,
}: ResizeHandleProps) {
  const origin = useRef<DragOrigin | null>(null);
  const [dragging, setDragging] = useState(false);
  const direction = panel === "left" ? 1 : -1;

  const endDrag = () => {
    origin.current = null;
    setDragging(false);
    onDraggingChange?.(false);
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.preventDefault();
    event.currentTarget.setPointerCapture(event.pointerId);
    origin.current = { x: event.clientX, width: value };
    setDragging(true);
    onDraggingChange?.(true);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const start = origin.current;
    if (!start) return;
    onResize(clamp(start.width + direction * (event.clientX - start.x), min, max));
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const keys: Record<string, number> = {
      ArrowLeft: value - direction * RESIZE_STEP,
      ArrowRight: value + direction * RESIZE_STEP,
      Home: min,
      End: max,
    };
    const next = keys[event.key];
    if (next === undefined) return;
    event.preventDefault();
    onResize(clamp(next, min, max));
  };

  return (
    <div
      // A focusable separator is an interactive widget (the WAI-ARIA
      // window splitter pattern); <hr> cannot take focus or children.
      // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
      role="separator"
      aria-orientation="vertical"
      aria-label={label}
      aria-controls={controls}
      aria-valuenow={Math.round(value)}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      data-dragging={dragging || undefined}
      className="group/resize relative z-10 flex w-1 shrink-0 cursor-col-resize touch-none justify-center outline-offset-0"
      onPointerDown={onPointerDown}
      onPointerMove={onPointerMove}
      onPointerUp={endDrag}
      onPointerCancel={endDrag}
      onDoubleClick={onReset}
      onKeyDown={onKeyDown}
    >
      <span
        aria-hidden
        className={cn(
          "my-6 w-px bg-transparent transition-colors [mask-image:linear-gradient(to_bottom,transparent,black_18%,black_82%,transparent)]",
          "group-hover/resize:bg-border-hover group-data-dragging/resize:bg-foreground-subtlest",
        )}
      />
    </div>
  );
}
