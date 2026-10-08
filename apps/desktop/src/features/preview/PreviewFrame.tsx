import brandIconLight from "@brand/blurrd-icon-light.svg";
import brandIcon from "@brand/blurrd-icon.svg";
import { PanelRightClose, PanelRightOpen } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Frame, FrameHeader, FrameTitle } from "@/features/shell/Frame";
import { INSPECTOR_ID } from "@/features/inspector/Inspector";
import { SHORTCUTS } from "@/lib/shortcuts";

/** Id of the inspector toggle in the preview header. */
export const INSPECTOR_TOGGLE_ID = "inspector-toggle";

export interface PreviewFrameProps {
  /** Whether the inspector (frame or sheet) is currently shown. */
  readonly inspectorShown: boolean;
  /** Shows or hides the inspector. */
  readonly onToggleInspector: () => void;
}

/**
 * The preview frame: a header with the inspector toggle and the stage
 * below it. Without a recording the stage shows the welcome message.
 */
export function PreviewFrame({ inspectorShown, onToggleInspector }: PreviewFrameProps) {
  const label = inspectorShown ? "Hide inspector" : "Show inspector";
  const { keys } = SHORTCUTS.toggleInspector;

  return (
    <Frame aria-labelledby="preview-title" className="flex-1">
      <FrameHeader>
        <FrameTitle id="preview-title">Preview</FrameTitle>
        <div className="flex-1" />
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-md"
              id={INSPECTOR_TOGGLE_ID}
              aria-label={label}
              aria-controls={INSPECTOR_ID}
              aria-expanded={inspectorShown}
              className="text-foreground-subtle hover:text-foreground"
              onClick={onToggleInspector}
            >
              {inspectorShown ? <PanelRightClose /> : <PanelRightOpen />}
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            {label}
            <Kbd>{keys.join("+")}</Kbd>
          </TooltipContent>
        </Tooltip>
      </FrameHeader>
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
        <img src={brandIcon} alt="" className="size-16 rounded-2xl dark:hidden" />
        <img src={brandIconLight} alt="" className="hidden size-16 rounded-2xl dark:block" />
        <div className="flex flex-col gap-1">
          <h1 className="text-ui-lg font-medium">Blur player names in your clips</h1>
          <p className="text-ui-base text-foreground-subtle">
            Everything runs on this PC. Nothing is uploaded.
          </p>
        </div>
      </div>
    </Frame>
  );
}
