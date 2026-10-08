import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";

import { MaximizeIcon, RestoreIcon } from "@/components/icons/window";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { logWarning } from "@/lib/log";
import { cn } from "@/lib/utils";

/** Click handler that runs a window call and logs its failure. */
function run(action: string, call: () => Promise<void>) {
  return () => {
    call().catch((error: unknown) => logWarning(`${action} failed`, error));
  };
}

/**
 * Minimize, maximize/restore and close buttons of the custom title bar
 * (the window has no native decorations, decision D6).
 *
 * The maximize button follows the real window state, which also changes
 * through double-clicks on the title bar, drag-to-edge snapping and
 * Win+arrow keys, so it listens to resize events instead of tracking its
 * own clicks.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const sync = () => {
      appWindow
        .isMaximized()
        .then((value) => {
          if (!disposed) setMaximized(value);
        })
        .catch((error: unknown) => logWarning("reading the window state failed", error));
    };

    sync();
    appWindow
      .onResized(sync)
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error: unknown) => logWarning("listening to window resizes failed", error));

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const appWindow = getCurrentWindow();

  return (
    <div className="flex items-center gap-0.5">
      <CaptionButton
        label="Minimize"
        onClick={run("minimizing the window", () => appWindow.minimize())}
      >
        <Minus />
      </CaptionButton>
      <CaptionButton
        label={maximized ? "Restore" : "Maximize"}
        onClick={run("maximizing the window", () => appWindow.toggleMaximize())}
      >
        {maximized ? <RestoreIcon /> : <MaximizeIcon />}
      </CaptionButton>
      <CaptionButton
        label="Close"
        className="hover:bg-destructive hover:text-destructive-foreground"
        onClick={run("closing the window", () => appWindow.close())}
      >
        <X />
      </CaptionButton>
    </div>
  );
}

interface CaptionButtonProps {
  readonly label: string;
  readonly onClick: () => void;
  readonly className?: string;
  readonly children: ReactNode;
}

/**
 * One window button: a 28 px ghost icon button with an 8 px radius and a
 * tooltip below it. Secondary text turns primary on hover.
 */
function CaptionButton({ label, onClick, className, children }: CaptionButtonProps) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-md"
          aria-label={label}
          className={cn("text-foreground-subtle hover:text-foreground", className)}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">{label}</TooltipContent>
    </Tooltip>
  );
}
