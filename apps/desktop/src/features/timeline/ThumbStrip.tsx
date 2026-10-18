import { memo } from "react";

import { useSchemeImage } from "@/hooks/useSchemeImage";
import { thumbnailUrl } from "@/ipc/media";

/** Width of one thumbnail slot in pixels (16:9 at the strip height). */
const SLOT = 64;

function Thumb({ url, left }: { readonly url: string; readonly left: number }) {
  const image = useSchemeImage(url);
  return image.data ? (
    <img
      src={image.data}
      alt=""
      draggable={false}
      className="absolute top-0 h-full object-cover"
      style={{ left, width: SLOT }}
    />
  ) : (
    <div className="absolute top-0 h-full bg-surface" style={{ left, width: SLOT - 1 }} />
  );
}

/** The thumbnail nearest to the hovered frame, in the timeline's hover card. */
export function HoverThumb({
  projectId,
  index,
}: {
  readonly projectId: string;
  readonly index: number;
}) {
  const image = useSchemeImage(thumbnailUrl(projectId, index));
  return (
    <div className="mb-1 h-[72px] w-32 overflow-hidden rounded-sm bg-video-stage">
      {image.data && <img src={image.data} alt="" className="size-full object-cover" />}
    </div>
  );
}

export interface ThumbStripProps {
  readonly projectId: string;
  /** Frames between two thumbnails and their number. */
  readonly every: number;
  readonly count: number;
  /** Visible frames and the strip's width in pixels. */
  readonly start: number;
  readonly end: number;
  readonly width: number;
  readonly height: number;
}

/**
 * The optional thumbnail strip of the timeline (Settings › Appearance ›
 * "Timeline thumbnails"): one backend thumbnail per 64 px slot, the one
 * nearest to the slot's middle frame.
 */
export const ThumbStrip = memo(function ThumbStrip({
  projectId,
  every,
  count,
  start,
  end,
  width,
  height,
}: ThumbStripProps) {
  const slots = Math.max(1, Math.ceil(width / SLOT));
  return (
    <div aria-hidden className="relative overflow-hidden bg-video-stage" style={{ width, height }}>
      {Array.from({ length: slots }, (_, slot) => {
        const frame = start + ((slot + 0.5) * SLOT * (end - start)) / width;
        const index = Math.min(Math.max(Math.round(frame / every), 0), count - 1);
        return <Thumb key={slot} url={thumbnailUrl(projectId, index)} left={slot * SLOT} />;
      })}
    </div>
  );
});
