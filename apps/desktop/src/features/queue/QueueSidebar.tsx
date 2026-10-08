import { Badge } from "@/components/ui/badge";
import { cn } from "@/lib/utils";

export interface QueueSidebarProps {
  /** Whether the sidebar is shown. */
  readonly open: boolean;
  /** Width while open, already clamped to the window. */
  readonly width: number;
  /** Disable the width animation (while a resize handle is dragged). */
  readonly resizing: boolean;
}

/** Id of the sidebar element, referenced by its toggle and resize handle. */
export const QUEUE_SIDEBAR_ID = "queue-sidebar";

/**
 * The queue sidebar. It sits directly on the window shell (no own fill)
 * and collapses to zero width with a 200 ms width and opacity transition;
 * the content keeps its open width while animating, so nothing reflows.
 * When collapsed it is `inert`, so focus skips it.
 */
export function QueueSidebar({ open, width, resizing }: QueueSidebarProps) {
  return (
    <aside
      id={QUEUE_SIDEBAR_ID}
      aria-label="Queue"
      inert={!open}
      className={cn(
        "flex min-h-0 shrink-0 flex-col overflow-hidden",
        !resizing && "transition-[width,opacity] duration-200 ease-out",
        !open && "opacity-0",
      )}
      style={{ width: open ? width : 0 }}
    >
      <div className="flex min-h-0 flex-1 flex-col pb-1 pl-2" style={{ width }}>
        <div className="flex h-10 shrink-0 items-center gap-2 px-2.5">
          <h2 className="text-ui-base font-medium">Queue</h2>
          <Badge aria-label="0 recordings">0</Badge>
        </div>
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-1 px-4 text-center">
          <p className="text-ui-base font-medium">No recordings yet</p>
          <p className="text-ui-sm text-foreground-subtle">
            Recordings you add are listed here with their status.
          </p>
        </div>
      </div>
    </aside>
  );
}
