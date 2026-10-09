import { memo, useId } from "react";

import type { FrameRange } from "@/bindings/FrameRange";
import type { VerifyReport } from "@/bindings/VerifyReport";
import { formatCount } from "@/lib/media";

import { frameToX, type TimelineView } from "./timeline-model";

export interface VerifyLaneProps {
  /** The last report, or `null` before the first verification. */
  readonly report: VerifyReport | null;
  /** Ranges changed by edits since the report (verified again before export). */
  readonly unverified: readonly FrameRange[];
  readonly view: TimelineView;
  readonly width: number;
  readonly height: number;
}

/**
 * The Verify lane (design 8.1): green where OCR found the output clean,
 * hatched where nothing has been verified yet, red over the frames of open
 * leaks. The leak markers above it are buttons of the timeline.
 */
export const VerifyLane = memo(function VerifyLane({
  report,
  unverified,
  view,
  width,
  height,
}: VerifyLaneProps) {
  const hatch = useId();
  const span = ([f0, f1]: FrameRange) => {
    const x0 = frameToX(f0, view, width);
    const x1 = frameToX(f1 + 1, view, width);
    return { x: x0, width: Math.max(1, x1 - x0) };
  };
  return (
    <svg aria-hidden width={width} height={height} className="block">
      <defs>
        <pattern
          id={hatch}
          width="6"
          height="6"
          patternUnits="userSpaceOnUse"
          patternTransform="rotate(45)"
        >
          <rect width="6" height="6" className="fill-surface" />
          <line
            x1="0"
            y1="0"
            x2="0"
            y2="6"
            strokeWidth="2"
            className="stroke-foreground-subtlest"
          />
        </pattern>
      </defs>
      {report === null ? (
        <rect width={width} height={height} fill={`url(#${hatch})`} data-verify="none" />
      ) : (
        <>
          <rect width={width} height={height} className="fill-success/45" data-verify="clean" />
          {unverified.map((range) => (
            <rect
              key={range.join("-")}
              {...span(range)}
              height={height}
              fill={`url(#${hatch})`}
              data-verify="unverified"
            />
          ))}
          {report.leaks
            .filter((leak) => leak.status === "Open")
            .map((leak) => (
              <rect
                key={`${leak.frame_range.join("-")}-${leak.rect.join(",")}`}
                {...span(leak.frame_range)}
                height={height}
                className="fill-destructive"
                data-verify="leak"
              />
            ))}
        </>
      )}
    </svg>
  );
});

/** A sentence for screen readers that sums up the Verify lane. */
export function verifySummary(
  report: VerifyReport | null,
  unverified: readonly FrameRange[],
): string {
  if (report === null) return "Not verified yet.";
  const { open, fixed } = report.summary;
  const parts = [
    open === 0 ? "No open leaks" : `${formatCount(open)} open ${open === 1 ? "leak" : "leaks"}`,
    `${formatCount(fixed)} fixed`,
  ];
  if (unverified.length > 0) {
    const ranges = unverified.map(([f0, f1]) => `${formatCount(f0)}–${formatCount(f1)}`).join(", ");
    parts.push(`frames ${ranges} not verified since the last edit`);
  }
  return `${parts.join(", ")}.`;
}
