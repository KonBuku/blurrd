import { memo } from "react";

import type { DensityLane } from "@/bindings/DensityLane";
import type { RegionFamily } from "@/lib/regions";
import { REGIONS } from "@/lib/regions";

import type { TimelineView } from "./timeline-model";

// Full class names, so that Tailwind finds them in the source.
const BAR_CLASSES: Readonly<Record<RegionFamily, string>> = {
  killfeed: "fill-region-killfeed",
  banner: "fill-region-banner",
  voice: "fill-region-voice",
  chat: "fill-region-chat",
  manual: "fill-region-manual",
  screens: "fill-region-screens",
};
const SURFACE_CLASSES: Readonly<Record<RegionFamily, string>> = {
  killfeed: "fill-region-killfeed-surface",
  banner: "fill-region-banner-surface",
  voice: "fill-region-voice-surface",
  chat: "fill-region-chat-surface",
  manual: "fill-region-manual-surface",
  screens: "fill-region-screens-surface",
};

/**
 * Bars of one lane as an SVG path: per pixel column the largest share of
 * frames with a box among the buckets under it, as a bar from the bottom
 * (at least 2 px when any frame has a box, so single frames stay visible).
 */
export function lanePath(
  coverage: readonly number[],
  frameCount: number,
  view: TimelineView,
  width: number,
  height: number,
): string {
  const buckets = coverage.length;
  const span = view.end - view.start;
  const parts: string[] = [];
  for (let x = 0; x < Math.floor(width); x += 1) {
    const f0 = view.start + (x / width) * span;
    const f1 = view.start + ((x + 1) / width) * span;
    const b0 = Math.floor((f0 / frameCount) * buckets);
    const b1 = Math.max(b0 + 1, Math.ceil((f1 / frameCount) * buckets));
    let value = 0;
    for (let b = b0; b < b1 && b < buckets; b += 1) value = Math.max(value, coverage[b] ?? 0);
    if (value <= 0) continue;
    const bar = Math.max(2, Math.round(value * height));
    parts.push(`M${x} ${height - bar}h1v${bar}h-1z`);
  }
  return parts.join("");
}

export interface DensityLanesProps {
  readonly lanes: readonly DensityLane[];
  readonly frameCount: number;
  readonly view: TimelineView;
  /** Width in pixels. */
  readonly width: number;
  readonly laneHeight: number;
  readonly gap: number;
}

/**
 * The region lanes of the timeline (design 8.1), one row per region with
 * boxes. Memoised: the playhead moves every frame, the lanes only with the
 * view.
 */
export const DensityLanes = memo(function DensityLanes({
  lanes,
  frameCount,
  view,
  width,
  laneHeight,
  gap,
}: DensityLanesProps) {
  return (
    <svg aria-hidden width={width} height={lanes.length * (laneHeight + gap)} className="block">
      {lanes.map((lane, row) => {
        const { family } = REGIONS[lane.region];
        return (
          <g key={lane.region} transform={`translate(0 ${row * (laneHeight + gap)})`}>
            <rect width={width} height={laneHeight} className={SURFACE_CLASSES[family]} />
            <path
              d={lanePath(lane.coverage, frameCount, view, width, laneHeight)}
              className={BAR_CLASSES[family]}
              data-region={lane.region}
            />
          </g>
        );
      })}
    </svg>
  );
});
