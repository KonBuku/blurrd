import { memo } from "react";

import { frameToX, tickLabel, tickStep, type TimelineView } from "./timeline-model";

export interface TimelineRulerProps {
  readonly view: TimelineView;
  readonly width: number;
  readonly height: number;
  /** Frames per second, to place ticks in time. */
  readonly fps: number;
}

/** The ruler: labelled ticks at a step that keeps them at least 72 px apart. */
export const TimelineRuler = memo(function TimelineRuler({
  view,
  width,
  height,
  fps,
}: TimelineRulerProps) {
  const step = tickStep(view, width, fps);
  const first = Math.ceil(view.start / fps / step);
  const last = Math.floor(view.end / fps / step);
  const ticks: number[] = [];
  for (let i = first; i <= last && ticks.length < 200; i += 1) ticks.push(i * step);
  return (
    <div aria-hidden className="relative overflow-hidden" style={{ height }}>
      {ticks.map((seconds) => (
        <div
          key={seconds}
          className="absolute top-0 h-full border-l border-border pl-1 font-mono text-ui-2xs leading-none text-foreground-subtle tabular-nums"
          style={{ left: frameToX(seconds * fps, view, width) }}
        >
          <span className="relative top-0.5">{tickLabel(seconds, step)}</span>
        </div>
      ))}
    </div>
  );
});
