import type { VerifyReport } from "@/bindings/VerifyReport";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { REGIONS, type RegionFamily } from "@/lib/regions";
import type { StageBox } from "@/lib/stage";
import { cn } from "@/lib/utils";
import { usePreviewStore } from "@/store/preview";

import { overlayItems, type OverlayItem } from "./overlay-model";
import { usePlaybackStore } from "./playback";
import type { LoadedProject } from "./useProject";

// Full class names, so that Tailwind finds them in the source.
const FAMILY_CLASSES: Readonly<Record<RegionFamily, string>> = {
  killfeed: "border-region-killfeed bg-region-killfeed-surface",
  banner: "border-region-banner bg-region-banner-surface",
  voice: "border-region-voice bg-region-voice-surface",
  chat: "border-region-chat bg-region-chat-surface",
  manual: "border-region-manual bg-region-manual-surface",
  screens: "border-region-screens bg-region-screens-surface",
};

/** Stroke and fill of an item; a halo in the panel colour keeps it visible on any video. */
function itemClass(item: OverlayItem, selected: boolean): string {
  if (selected) return "border-2 border-accent-strong bg-accent-strong/10";
  if (item.kind === "leak") {
    return item.open
      ? "border border-dashed border-destructive bg-destructive/10"
      : "border border-dashed border-warning";
  }
  const family = item.region === null ? "manual" : REGIONS[item.region].family;
  if (item.kind === "unbound")
    return cn("border border-dashed", FAMILY_CLASSES[family], "bg-transparent");
  return cn("border", FAMILY_CLASSES[family]);
}

export interface BoxOverlayProps {
  readonly data: LoadedProject;
  readonly report: VerifyReport | null;
  /** Where the frame is on the stage. */
  readonly box: StageBox;
}

/**
 * The box overlay over the preview frame (design 8.1): every blurred box
 * with a 1 px stroke and 10 % fill in its region colour, leaks of the last
 * verification with a dashed destructive stroke, and inactive (unbound)
 * edits dashed. Positions are source rectangles scaled with the stage box
 * shared with the canvas, so they line up at every zoom. Each item is a
 * button with its description as name and tooltip; clicking selects it.
 */
export function BoxOverlay({ data, report, box }: BoxOverlayProps) {
  const frame = usePlaybackStore((state) => state.frame);
  const selected = usePreviewStore((state) => state.selected);
  const select = usePreviewStore((state) => state.select);
  const items = overlayItems(frame, data.index, data.enabled, data.tracks, report);

  return (
    <div className="pointer-events-none absolute inset-0" data-testid="box-overlay">
      {items.map((item) => {
        const [x0, y0, x1, y1] = item.rect;
        const isSelected = item.key === selected;
        return (
          <Tooltip key={item.key}>
            <TooltipTrigger asChild>
              <button
                type="button"
                data-no-pan
                data-overlay={item.kind}
                aria-label={item.label}
                aria-pressed={isSelected}
                className={cn(
                  "pointer-events-auto absolute rounded-[1px] shadow-[0_0_0_1px_color-mix(in_oklab,var(--color-background)_70%,transparent)] outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  itemClass(item, isSelected),
                )}
                style={{
                  left: box.left + x0 * box.scale,
                  top: box.top + y0 * box.scale,
                  width: (x1 - x0) * box.scale,
                  height: (y1 - y0) * box.scale,
                }}
                onClick={() => select(isSelected ? null : item.key)}
              />
            </TooltipTrigger>
            <TooltipContent side="top">{item.label}</TooltipContent>
          </Tooltip>
        );
      })}
    </div>
  );
}
