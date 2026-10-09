import type { KeyboardEvent, PointerEvent } from "react";

import type { StageBox } from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";

/** Keyboard step of the divider, as a fraction of the frame width. */
const STEP = 0.02;

/**
 * The Split divider (design 8.1): a 2 px white line with a 24 px round
 * grip, dragged with the pointer or moved with the arrow keys (a WAI-ARIA
 * slider); "Before" is shown left of it and "After" right of it.
 */
export function SplitDivider({ box }: { readonly box: StageBox }) {
  const split = usePreviewStore((state) => state.split);
  const setSplit = usePreviewStore((state) => state.setSplit);
  const x = box.left + split * box.width;

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    const element = event.currentTarget;
    element.setPointerCapture(event.pointerId);
    const stage = element.parentElement?.getBoundingClientRect();
    const move = (e: globalThis.PointerEvent) => {
      if (!stage) return;
      setSplit((e.clientX - stage.left - box.left) / box.width);
    };
    const up = () => {
      element.removeEventListener("pointermove", move);
      element.removeEventListener("pointerup", up);
      element.removeEventListener("pointercancel", up);
    };
    element.addEventListener("pointermove", move);
    element.addEventListener("pointerup", up);
    element.addEventListener("pointercancel", up);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const moves: Record<string, number> = {
      ArrowLeft: split - STEP,
      ArrowRight: split + STEP,
      Home: 0,
      End: 1,
    };
    const next = moves[event.key];
    if (next === undefined) return;
    event.preventDefault();
    event.stopPropagation();
    setSplit(next);
  };

  const pill = "rounded-full bg-black/60 px-2 py-0.5 text-ui-xs text-white";
  return (
    <>
      <span
        className={`pointer-events-none absolute top-2 ${pill}`}
        style={{ left: Math.max(x - 64, 8) }}
      >
        Before
      </span>
      <span className={`pointer-events-none absolute top-2 ${pill}`} style={{ left: x + 12 }}>
        After
      </span>
      <div
        // A full-height divider at the exact split position; a native range
        // input cannot draw the line across the frame.
        // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
        role="slider"
        data-no-pan
        tabIndex={0}
        aria-label="Split position"
        aria-orientation="horizontal"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(split * 100)}
        aria-valuetext={`${Math.round(split * 100)} % before`}
        className="group absolute top-0 bottom-0 flex w-6 -translate-x-1/2 cursor-ew-resize justify-center outline-none"
        style={{ left: x }}
        onPointerDown={onPointerDown}
        onKeyDown={onKeyDown}
      >
        <div className="h-full w-0.5 bg-white shadow-[0_0_2px_rgba(0,0,0,0.6)]" />
        <div className="absolute top-1/2 size-6 -translate-y-1/2 rounded-full border-2 border-white bg-black/40 group-focus-visible:ring-2 group-focus-visible:ring-ring" />
      </div>
    </>
  );
}
