import brandIcon from "@brand/blurrd-icon-small.svg";
import { PanelLeft } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { QUEUE_SIDEBAR_ID } from "@/features/queue/QueueSidebar";
import { SHORTCUTS } from "@/lib/shortcuts";

import { TitleBarActions } from "./TitleBarActions";
import { WindowControls } from "./WindowControls";

export interface TitleBarProps {
  /** Whether the queue sidebar is shown. */
  readonly sidebarOpen: boolean;
  /** Shows or hides the queue sidebar. */
  readonly onToggleSidebar: () => void;
}

/**
 * The custom title bar (decision D6), 48 px tall on the window shell
 * colour. The bar and its empty areas are `data-tauri-drag-region`:
 * dragging moves the window with native drag-to-edge snapping, and a
 * double-click maximizes or restores it. Buttons inside the bar are not
 * drag regions.
 *
 * The app mark on the left doubles as the queue sidebar toggle: on hover
 * and keyboard focus it crossfades to the sidebar icon (150 ms). The theme
 * menu and the settings button sit left of the window controls.
 */
export function TitleBar({ sidebarOpen, onToggleSidebar }: TitleBarProps) {
  const label = sidebarOpen ? "Hide queue" : "Show queue";
  const { keys } = SHORTCUTS.toggleQueue;

  return (
    <header
      data-tauri-drag-region
      className="flex h-12 shrink-0 items-center gap-2 px-2 select-none"
    >
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-md"
            aria-label={label}
            aria-controls={QUEUE_SIDEBAR_ID}
            aria-expanded={sidebarOpen}
            className="group/toggle relative text-foreground-subtle hover:text-foreground"
            onClick={onToggleSidebar}
          >
            <img
              src={brandIcon}
              alt=""
              className="size-5 transition-opacity duration-150 group-hover/toggle:opacity-0 group-focus-visible/toggle:opacity-0"
            />
            <PanelLeft className="absolute opacity-0 transition-opacity duration-150 group-hover/toggle:opacity-100 group-focus-visible/toggle:opacity-100" />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          {label}
          <Kbd>{keys.join("+")}</Kbd>
        </TooltipContent>
      </Tooltip>
      <span data-tauri-drag-region className="text-ui-base font-medium">
        blurrd
      </span>
      <div data-tauri-drag-region className="h-full min-w-0 flex-1" />
      <TitleBarActions />
      <div aria-hidden className="mx-1 h-4 w-px bg-border" />
      <WindowControls />
    </header>
  );
}
