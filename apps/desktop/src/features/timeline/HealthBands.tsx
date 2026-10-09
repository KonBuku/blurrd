import type { Region } from "@/bindings/Region";
import type { HealthSpan } from "@/ipc/commands";

import { frameToX, type TimelineView } from "./timeline-model";

export interface HealthBandsProps {
  readonly spans: readonly HealthSpan[];
  /** Regions of the density lanes, top to bottom. */
  readonly lanes: readonly Region[];
  readonly view: TimelineView;
  readonly width: number;
  /** Top of the first lane and the distance between lanes, in pixels. */
  readonly top: number;
  readonly rowHeight: number;
  readonly laneHeight: number;
}

/**
 * Health-warning spans (`RegionHealth`): where a region's detection fell
 * back to its safe slot boxes, a warning-coloured outline over that span
 * of the region's lane.
 */
export function HealthBands({
  spans,
  lanes,
  view,
  width,
  top,
  rowHeight,
  laneHeight,
}: HealthBandsProps) {
  return (
    <>
      {spans.map((span) => {
        const row = lanes.indexOf(span.region);
        if (row < 0) return null;
        const x0 = frameToX(span.f0, view, width);
        const x1 = frameToX(span.f1 + 1, view, width);
        if (x1 < 0 || x0 > width) return null;
        return (
          <div
            key={`${span.region}-${span.f0}`}
            data-health={span.region}
            className="absolute rounded-[2px] border border-warning bg-warning/20"
            style={{
              left: x0,
              width: Math.max(2, x1 - x0),
              top: top + row * rowHeight,
              height: laneHeight,
            }}
          />
        );
      })}
    </>
  );
}
