import { memo, useCallback, useEffect, useState } from "react";

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

interface OverlayButtonProps {
  readonly item: OverlayItem;
  readonly selected: boolean;
  readonly box: StageBox;
  readonly onSelect: (key: string | null) => void;
}

/** Everything about an item that changes how it is drawn. */
function itemSignature(item: OverlayItem): string {
  return `${item.key}|${item.kind}|${item.rect.join(",")}|${item.label}|${String(item.open)}|${item.region ?? ""}`;
}

/**
 * The overlay items of the presented frame. Subscribes to the playback
 * store itself and only updates when the items change, so that playback
 * does not re-render the overlay on frames that look the same.
 */
function useOverlayItems(data: LoadedProject, report: VerifyReport | null): readonly OverlayItem[] {
  const compute = useCallback(() => {
    const items = overlayItems(
      usePlaybackStore.getState().frame,
      data.index,
      data.enabled,
      data.tracks,
      report,
    );
    return { items, key: items.map(itemSignature).join(";") };
  }, [data, report]);
  const [state, setState] = useState(compute);
  useEffect(() => {
    const update = () => {
      const next = compute();
      setState((previous) => (previous.key === next.key ? previous : next));
    };
    update();
    return usePlaybackStore.subscribe(update);
  }, [compute]);
  return state.items;
}

/** Whether two renders of an item would draw the same. */
function sameButton(a: OverlayButtonProps, b: OverlayButtonProps): boolean {
  return (
    itemSignature(a.item) === itemSignature(b.item) &&
    a.selected === b.selected &&
    a.box.left === b.box.left &&
    a.box.top === b.box.top &&
    a.box.scale === b.box.scale &&
    a.onSelect === b.onSelect
  );
}

/**
 * One overlay item as a button with its tooltip. Memoised: during
 * playback the overlay re-renders every frame, but most items stay put.
 */
const OverlayButton = memo(function OverlayButton({
  item,
  selected,
  box,
  onSelect,
}: OverlayButtonProps) {
  const [x0, y0, x1, y1] = item.rect;
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <button
          type="button"
          data-no-pan
          data-overlay={item.kind}
          aria-label={item.label}
          aria-pressed={selected}
          className={cn(
            "pointer-events-auto absolute rounded-[1px] shadow-[0_0_0_1px_color-mix(in_oklab,var(--color-background)_70%,transparent)] outline-none focus-visible:ring-2 focus-visible:ring-ring",
            itemClass(item, selected),
          )}
          style={{
            left: box.left + x0 * box.scale,
            top: box.top + y0 * box.scale,
            width: (x1 - x0) * box.scale,
            height: (y1 - y0) * box.scale,
          }}
          onClick={() => onSelect(selected ? null : item.key)}
        />
      </TooltipTrigger>
      <TooltipContent side="top">{item.label}</TooltipContent>
    </Tooltip>
  );
}, sameButton);

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
  const selected = usePreviewStore((state) => state.selected);
  const select = usePreviewStore((state) => state.select);
  const items = useOverlayItems(data, report);

  return (
    <div className="pointer-events-none absolute inset-0" data-testid="box-overlay">
      {items.map((item) => (
        <OverlayButton
          key={item.key}
          item={item}
          selected={item.key === selected}
          box={box}
          onSelect={select}
        />
      ))}
    </div>
  );
}
