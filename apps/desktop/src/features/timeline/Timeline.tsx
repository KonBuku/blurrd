import { Triangle, TriangleAlert } from "lucide-react";
import {
  useEffect,
  useEffectEvent,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type PointerEvent,
} from "react";

import type { LeakStatus } from "@/bindings/LeakStatus";
import type { Region } from "@/bindings/Region";
import type { VerifyReport } from "@/bindings/VerifyReport";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePlaybackStore, useControllerStore } from "@/features/preview/playback";
import type { LoadedProject } from "@/features/preview/useProject";
import { useElementSize } from "@/hooks/useElementSize";
import { formatTimecode, type FrameClock } from "@/lib/frames";
import { formatCount } from "@/lib/media";
import { REGIONS } from "@/lib/regions";
import { cn } from "@/lib/utils";
import { usePreviewStore } from "@/store/preview";
import { useSettingsStore } from "@/store/settings";

import { DensityLanes } from "./DensityLanes";
import { HealthBands } from "./HealthBands";
import { HoverThumb, ThumbStrip } from "./ThumbStrip";
import { TimelineRuler } from "./TimelineRuler";
import {
  frameToX,
  fullView,
  panView,
  revealFrame,
  xToFrame,
  zoomView,
  type TimelineView,
} from "./timeline-model";
import { useDensity } from "./useTimelineData";
import { VerifyLane, verifySummary } from "./VerifyLane";

/** Row heights in pixels. */
const RULER = 18;
const LANE = 12;
const GAP = 2;
const VERIFY = 12;
const THUMBS = 36;

/** Box counts per region on `frame`, for the hover card. */
function regionsAt(data: LoadedProject, frame: number): string {
  const counts = new Map<Region, number>();
  for (const b of data.index.boxesAt(frame, data.enabled))
    counts.set(b.region, (counts.get(b.region) ?? 0) + 1);
  if (counts.size === 0) return "No boxes";
  return [...counts].map(([region, n]) => `${REGIONS[region].label} ${n}`).join(" · ");
}

export interface TimelineProps {
  readonly data: LoadedProject;
  readonly report: VerifyReport | null;
}

/**
 * The timeline under the preview (design 8.1): ruler, one density lane
 * per region with boxes, health-warning spans, the Verify lane with leak
 * markers, the optional thumbnail strip and the playhead. Click or drag
 * scrubs frame-accurately; Ctrl+wheel zooms around the pointer, the wheel
 * pans; while playing, the view follows the playhead. The track is a
 * slider: arrows step frames, Shift+arrows and Page Up/Down one second,
 * Home/End the ends.
 */
export function Timeline({ data, report }: TimelineProps) {
  const { project, clock, tracks } = data;
  const count = clock.count;
  const fps = project.media.fps[1] > 0 ? project.media.fps[0] / project.media.fps[1] : 60;
  const controller = useControllerStore((state) => state.controller);
  const select = usePreviewStore((state) => state.select);
  const thumbsOn = useSettingsStore((state) => state.appearance.timeline_thumbs);
  const [base, setBase] = useState<TimelineView>(() => fullView(count));
  const [hover, setHover] = useState<{ x: number; frame: number } | null>(null);
  const trackRef = useRef<HTMLDivElement>(null);
  const [trackElement, setTrackElement] = useState<HTMLDivElement | null>(null);
  const { width } = useElementSize(trackRef);
  const density = useDensity(project.id, tracks.revision, count);
  const lanes = density.data?.lanes ?? [];
  const thumbs = thumbsOn ? project.thumbs : null;
  // The view follows the playhead whenever it leaves the visible range.
  // Only the view's start is taken from the store, so that the timeline
  // re-renders when the view moves, not on every frame.
  const start = usePlaybackStore(
    (state) => revealFrame(base, state.target ?? state.frame, count).start,
  );
  const length = base.end - base.start;
  const view = useMemo(() => ({ start, end: start + length }), [start, length]);
  const lanesHeight = lanes.length * (LANE + GAP);
  const verifyTop = RULER + lanesHeight;
  const height = verifyTop + VERIFY + GAP + (thumbs ? THUMBS : 0);
  const leaks = (report?.leaks ?? []).map((leak, index) => ({ leak, index }));

  const seekAt = (clientX: number) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || !controller) return;
    controller.seek(xToFrame(clientX - rect.left, view, width, count));
  };

  const onPointerDown = (event: PointerEvent<HTMLDivElement>) => {
    if (event.button !== 0) return;
    event.currentTarget.setPointerCapture(event.pointerId);
    seekAt(event.clientX);
  };

  const onPointerMove = (event: PointerEvent<HTMLDivElement>) => {
    const rect = event.currentTarget.getBoundingClientRect();
    const x = event.clientX - rect.left;
    setHover({ x, frame: xToFrame(x, view, width, count) });
    if (event.currentTarget.hasPointerCapture(event.pointerId)) seekAt(event.clientX);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const { frame, target } = usePlaybackStore.getState();
    const current = target ?? frame;
    const second = Math.max(1, Math.round(fps));
    const step = event.shiftKey ? second : 1;
    const moves: Record<string, number> = {
      ArrowLeft: current - step,
      ArrowRight: current + step,
      PageUp: current - second,
      PageDown: current + second,
      Home: 0,
      End: count - 1,
    };
    const next = moves[event.key];
    if (next === undefined || !controller || event.ctrlKey || event.altKey) return;
    event.preventDefault();
    controller.seek(next);
  };

  // Ctrl+wheel zooms, the wheel pans (native listener: it must prevent the page zoom).
  const onWheel = useEffectEvent((event: WheelEvent, x: number) => {
    const span = view.end - view.start;
    if (event.ctrlKey) {
      event.preventDefault();
      const anchor = view.start + (x / Math.max(width, 1)) * span;
      setBase(zoomView(view, anchor, Math.exp(-event.deltaY / 300), count));
    } else if (span < count) {
      event.preventDefault();
      const delta =
        (event.deltaX !== 0 ? event.deltaX : event.deltaY) * (span / Math.max(width, 1));
      setBase(panView(view, delta, count));
    }
  });
  useEffect(() => {
    if (!trackElement) return undefined;
    const wheel = (event: WheelEvent) =>
      onWheel(event, event.clientX - trackElement.getBoundingClientRect().left);
    trackElement.addEventListener("wheel", wheel, { passive: false });
    return () => trackElement.removeEventListener("wheel", wheel);
  }, [trackElement]);

  return (
    <div className="flex shrink-0 border-t border-border select-none" data-testid="timeline">
      <div className="flex w-24 shrink-0 flex-col border-r border-border pl-3 text-ui-xs text-foreground-subtle">
        <div style={{ height: RULER }} />
        {lanes.map((lane) => {
          const unhealthy = tracks.health.filter((span) => span.region === lane.region).length;
          return (
            <div
              key={lane.region}
              className="flex items-center gap-1.5"
              style={{ height: LANE + GAP }}
            >
              <span
                className={cn(
                  "size-1.5 shrink-0 rounded-full",
                  DOT_CLASSES[REGIONS[lane.region].family],
                )}
              />
              <span className="truncate">{REGIONS[lane.region].label}</span>
              {unhealthy > 0 && (
                <Tooltip>
                  <TooltipTrigger asChild>
                    <button
                      type="button"
                      aria-label={`${REGIONS[lane.region].label}: detection unreliable in ${unhealthy} ${unhealthy === 1 ? "span" : "spans"}`}
                      className="shrink-0 text-warning outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                      <TriangleAlert className="size-2.5" />
                    </button>
                  </TooltipTrigger>
                  <TooltipContent side="right">
                    Detection was unreliable in the outlined spans; the region's safe slot boxes are
                    blurred there.
                  </TooltipContent>
                </Tooltip>
              )}
            </div>
          );
        })}
        <div className="flex items-center" style={{ height: VERIFY + GAP }}>
          Verify
        </div>
      </div>
      <div
        ref={(element) => {
          trackRef.current = element;
          setTrackElement(element);
        }}
        className="relative min-w-0 flex-1"
        style={{ height }}
      >
        <ScrubSlider
          clock={clock}
          describedBy={`${project.id}-verify`}
          onPointerDown={onPointerDown}
          onPointerMove={onPointerMove}
          onPointerLeave={() => setHover(null)}
          onKeyDown={onKeyDown}
        />
        <span id={`${project.id}-verify`} className="sr-only">
          {verifySummary(report, project.verify.unverified_ranges)}
        </span>
        <div className="pointer-events-none absolute inset-0">
          <TimelineRuler view={view} width={width} height={RULER} fps={fps} />
          <DensityLanes
            lanes={lanes}
            frameCount={count}
            view={view}
            width={width}
            laneHeight={LANE}
            gap={GAP}
          />
          <HealthBands
            spans={tracks.health}
            lanes={lanes.map((lane) => lane.region)}
            view={view}
            width={width}
            top={RULER}
            rowHeight={LANE + GAP}
            laneHeight={LANE}
          />
          <VerifyLane
            report={report}
            unverified={project.verify.unverified_ranges}
            view={view}
            width={width}
            height={VERIFY}
          />
          {thumbs && (
            <div style={{ marginTop: GAP }}>
              <ThumbStrip
                projectId={project.id}
                every={thumbs.every}
                count={thumbs.count}
                start={view.start}
                end={view.end}
                width={width}
                height={THUMBS}
              />
            </div>
          )}
          <Playhead clock={clock} view={view} width={width} />
        </div>
        {leaks.map(({ leak, index }) => {
          const x = frameToX(leak.frame_range[0], view, width);
          if (x < -6 || x > width + 6) return null;
          const label = `${LEAK_STATUS[leak.status].label} · ${leak.region ? REGIONS[leak.region].label : "outside the HUD"} · frame ${formatCount(leak.frame_range[0])}`;
          return (
            <Tooltip key={index}>
              <TooltipTrigger asChild>
                <button
                  type="button"
                  aria-label={label}
                  className={cn(
                    "absolute flex size-3 -translate-x-1/2 items-center justify-center outline-none focus-visible:ring-2 focus-visible:ring-ring",
                    LEAK_STATUS[leak.status].className,
                  )}
                  style={{ left: x, top: verifyTop }}
                  onClick={() => {
                    controller?.seek(leak.frame_range[0]);
                    select(`leak:${index}`);
                  }}
                >
                  <Triangle className="size-2.5 fill-current" strokeWidth={0} />
                </button>
              </TooltipTrigger>
              <TooltipContent side="top">{label}</TooltipContent>
            </Tooltip>
          );
        })}
        {hover && (
          <div
            className="pointer-events-none absolute bottom-full mb-1 -translate-x-1/2 rounded-md border border-popover-border bg-popover px-2 py-1 text-ui-xs whitespace-nowrap text-popover-foreground shadow-md"
            style={{ left: Math.min(Math.max(hover.x, 80), width - 80) }}
          >
            {project.thumbs && (
              <HoverThumb
                projectId={project.id}
                index={Math.min(
                  Math.round(hover.frame / project.thumbs.every),
                  project.thumbs.count - 1,
                )}
              />
            )}
            <span className="font-mono tabular-nums">
              {formatTimecode(clock.startOf(hover.frame))} · frame {formatCount(hover.frame)}
            </span>
            <br />
            {regionsAt(data, hover.frame)}
          </div>
        )}
      </div>
    </div>
  );
}

interface ScrubSliderProps {
  readonly clock: FrameClock;
  /** Id of the element that describes the Verify lane. */
  readonly describedBy: string;
  readonly onPointerDown: (event: PointerEvent<HTMLDivElement>) => void;
  readonly onPointerMove: (event: PointerEvent<HTMLDivElement>) => void;
  readonly onPointerLeave: () => void;
  readonly onKeyDown: (event: KeyboardEvent<HTMLDivElement>) => void;
}

/**
 * The focusable slider over the whole track that takes pointer and key
 * input; its value (the shown frame) is the only thing it renders.
 */
function ScrubSlider({ clock, describedBy, ...handlers }: ScrubSliderProps) {
  const frame = usePlaybackStore((state) => state.target ?? state.frame);
  return (
    <div
      // A range input cannot span the lanes, markers and playhead.
      // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
      role="slider"
      tabIndex={0}
      aria-label="Timeline"
      aria-valuemin={0}
      aria-valuemax={Math.max(clock.count - 1, 0)}
      aria-valuenow={frame}
      aria-valuetext={`${formatTimecode(clock.startOf(frame))}, frame ${formatCount(frame)}`}
      aria-describedby={describedBy}
      className="absolute inset-0 cursor-pointer outline-none focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
      {...handlers}
    />
  );
}

interface PlayheadProps {
  readonly clock: FrameClock;
  readonly view: TimelineView;
  readonly width: number;
}

/** The playhead with its time flag; the only part that moves every frame. */
function Playhead({ clock, view, width }: PlayheadProps) {
  const frame = usePlaybackStore((state) => state.target ?? state.frame);
  return (
    <div
      className="absolute top-0 bottom-0 w-px bg-ring"
      style={{ left: frameToX(frame, view, width) }}
      data-testid="playhead"
    >
      <span className="absolute -top-px left-1 rounded-sm bg-foreground px-1 font-mono text-ui-2xs text-background tabular-nums">
        {formatTimecode(clock.startOf(frame))}
      </span>
    </div>
  );
}

/** Name and colour of a leak marker per status. */
const LEAK_STATUS: Readonly<Record<LeakStatus, { label: string; className: string }>> = {
  Open: { label: "Open leak", className: "text-destructive" },
  Fixed: { label: "Fixed leak", className: "text-success" },
  Accepted: { label: "Accepted leak", className: "text-warning" },
  Info: { label: "Name outside the blurred regions", className: "text-warning" },
};

// Full class names, so that Tailwind finds them in the source.
const DOT_CLASSES = {
  killfeed: "bg-region-killfeed",
  banner: "bg-region-banner",
  voice: "bg-region-voice",
  chat: "bg-region-chat",
  manual: "bg-region-manual",
  screens: "bg-region-screens",
} as const;
