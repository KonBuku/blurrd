import { ChevronDown, SquareDashed, TriangleAlert } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ToggleGroup, type ToggleOption } from "@/components/ui/toggle-group";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ZOOM_LEVELS, type Zoom } from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";

import type { PreviewMode } from "./gl/blur-renderer";
import type { StageStatus } from "./VideoStage";

const MODES: readonly ToggleOption<PreviewMode>[] = [
  { value: "before", label: "Before" },
  { value: "after", label: "After" },
  { value: "split", label: "Split" },
];

/** Label of a zoom setting ("Fit", "200 %"). */
export function zoomLabel(zoom: Zoom): string {
  return zoom === "fit" ? "Fit" : `${Math.round(zoom * 100)} %`;
}

/**
 * The preview controls of the frame header (design 8.1): Before / After /
 * Split, Exact, the zoom menu, and badges for the proxy stream and a
 * preview without blur.
 */
export function PreviewToolbar({ status }: { readonly status: StageStatus }) {
  const mode = usePreviewStore((state) => state.mode);
  const setMode = usePreviewStore((state) => state.setMode);
  const showBoxes = usePreviewStore((state) => state.showBoxes);
  const toggleBoxes = usePreviewStore((state) => state.toggleBoxes);
  const exact = usePreviewStore((state) => state.exact);
  const toggleExact = usePreviewStore((state) => state.toggleExact);
  const zoom = usePreviewStore((state) => state.zoom);
  const setZoom = usePreviewStore((state) => state.setZoom);

  return (
    <div className="flex shrink-0 items-center gap-1.5">
      {status.proxy && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Badge variant="outline" tabIndex={0}>
              Proxy
            </Badge>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            Playing the preview copy made during analysis; the export uses the original.
          </TooltipContent>
        </Tooltip>
      )}
      {status.webglProblem !== null && (
        <Tooltip>
          <TooltipTrigger asChild>
            <Badge variant="warning" tabIndex={0}>
              <TriangleAlert />
              No blur preview
            </Badge>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            {status.webglProblem} The export still blurs every name.
          </TooltipContent>
        </Tooltip>
      )}
      <ToggleGroup
        size="sm"
        aria-label="Preview mode"
        options={MODES}
        value={mode}
        onValueChange={setMode}
        disabled={status.webglProblem !== null}
      />
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={showBoxes}
            className="aria-pressed:bg-selected aria-pressed:text-accent-foreground"
            onClick={toggleBoxes}
          >
            <SquareDashed />
            Boxes
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          Show the blurred boxes, leaks and inactive edits
        </TooltipContent>
      </Tooltip>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-pressed={exact}
            className="aria-pressed:bg-selected aria-pressed:text-accent-foreground"
            onClick={toggleExact}
          >
            Exact
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          When paused, show the export's own blur of the HUD regions
        </TooltipContent>
      </Tooltip>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Zoom: ${zoomLabel(zoom)}`}
            className="tabular-nums"
          >
            {zoomLabel(zoom)}
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuRadioGroup
            value={String(zoom)}
            onValueChange={(value) => setZoom(value === "fit" ? "fit" : Number(value), null)}
          >
            <DropdownMenuRadioItem value="fit">Fit</DropdownMenuRadioItem>
            {ZOOM_LEVELS.map((level) => (
              <DropdownMenuRadioItem key={level} value={String(level)}>
                {zoomLabel(level)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
    </div>
  );
}
