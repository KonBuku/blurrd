import { useEffect, useEffectEvent } from "react";

import {
  centerForAnchor,
  clampCenter,
  MAX_ZOOM,
  MIN_ZOOM,
  toSource,
  type Size,
  type StageBox,
} from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";

/** Pixels the pointer moves before a press becomes a pan. */
const DRAG_THRESHOLD = 3;

/** Whether the frame is larger than the stage along an axis. */
function zoomedIn(box: StageBox, stage: Size): boolean {
  return box.width > stage.width || box.height > stage.height;
}

/**
 * Zoom and pan input of the stage element: Ctrl+wheel zooms around the
 * pointer (a native, non-passive listener, so the page itself never
 * zooms); the wheel and a drag pan when the frame is larger than the
 * stage. Controls on the stage marked `data-no-pan` keep their drags.
 */
export function usePan(element: HTMLElement | null, box: StageBox, stage: Size, media: Size): void {
  const setZoom = usePreviewStore((state) => state.setZoom);
  const setCenter = usePreviewStore((state) => state.setCenter);

  const centerNow = () => toSource(box, { x: stage.width / 2, y: stage.height / 2 });

  const onWheel = useEffectEvent((event: WheelEvent, at: { x: number; y: number }) => {
    if (event.ctrlKey) {
      event.preventDefault();
      const scale = Math.min(
        Math.max(box.scale * Math.exp(-event.deltaY / 400), MIN_ZOOM),
        MAX_ZOOM,
      );
      const anchor = toSource(box, at);
      setZoom(scale, clampCenter(centerForAnchor(anchor, at, stage, scale), stage, media, scale));
      return;
    }
    if (!zoomedIn(box, stage)) return;
    event.preventDefault();
    const c = centerNow();
    const dx = (event.shiftKey ? event.deltaY : event.deltaX) / box.scale;
    const dy = (event.shiftKey ? 0 : event.deltaY) / box.scale;
    setCenter(clampCenter({ x: c.x + dx, y: c.y + dy }, stage, media, box.scale));
  });

  /** The drag start, or `null` when a press must not pan. */
  const startDrag = useEffectEvent((event: PointerEvent) => {
    if (!zoomedIn(box, stage) || (event.button !== 0 && event.button !== 1)) return null;
    if (event.target instanceof Element && event.target.closest("[data-no-pan]")) return null;
    return { x: event.clientX, y: event.clientY, center: centerNow() };
  });

  const dragTo = useEffectEvent(
    (start: { x: number; y: number; center: { x: number; y: number } }, e: PointerEvent) => {
      const next = {
        x: start.center.x - (e.clientX - start.x) / box.scale,
        y: start.center.y - (e.clientY - start.y) / box.scale,
      };
      setCenter(clampCenter(next, stage, media, box.scale));
    },
  );

  useEffect(() => {
    if (!element) return undefined;
    const wheel = (event: WheelEvent) => {
      const rect = element.getBoundingClientRect();
      onWheel(event, { x: event.clientX - rect.left, y: event.clientY - rect.top });
    };
    const down = (event: PointerEvent) => {
      const start = startDrag(event);
      if (!start) return;
      let dragging = false;
      const move = (e: PointerEvent) => {
        if (!dragging && Math.hypot(e.clientX - start.x, e.clientY - start.y) < DRAG_THRESHOLD)
          return;
        if (!dragging) element.setPointerCapture(event.pointerId);
        dragging = true;
        dragTo(start, e);
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
    element.addEventListener("wheel", wheel, { passive: false });
    element.addEventListener("pointerdown", down);
    return () => {
      element.removeEventListener("wheel", wheel);
      element.removeEventListener("pointerdown", down);
    };
  }, [element]);
}
