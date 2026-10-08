import { Ellipsis, FolderPlus, Plus, Search } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuShortcut,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { browseFiles, browseFolder } from "@/features/import/import";
import { formatCount } from "@/lib/media";
import { isFinished } from "@/lib/queue-status";
import { SHORTCUTS } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useQueueStore, visibleItems } from "@/store/queue";

import { QueueList } from "./QueueList";

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

/** The filter field appears once the queue is longer than this (design 8.1). */
export const FILTER_THRESHOLD = 8;

/**
 * The queue sidebar. It sits directly on the window shell (no own fill)
 * and collapses to zero width with a 200 ms width and opacity transition;
 * the content keeps its open width while animating, so nothing reflows.
 * When collapsed it is `inert`, so focus skips it.
 */
export function QueueSidebar({ open, width, resizing }: QueueSidebarProps) {
  const items = useQueueStore((state) => state.items);
  const filter = useQueueStore((state) => state.filter);
  const setFilter = useQueueStore((state) => state.setFilter);
  const showFilter = items.length > FILTER_THRESHOLD;
  const shown = visibleItems(items, showFilter ? filter : "");
  const count = formatCount(items.length);

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
      <div className="flex min-h-0 flex-1 flex-col gap-1 pb-1 pl-2" style={{ width }}>
        <div className="flex h-10 shrink-0 items-center gap-2 pr-1 pl-2.5">
          <h2 className="text-ui-base font-medium">Queue</h2>
          <Badge aria-label={items.length === 1 ? "1 recording" : `${count} recordings`}>
            {count}
          </Badge>
          <div className="flex-1" />
          <QueueActions />
        </div>
        {showFilter && (
          <div className="relative shrink-0 pr-1">
            <Search
              aria-hidden
              className="pointer-events-none absolute top-1/2 left-2 size-3 -translate-y-1/2 text-foreground-subtlest"
            />
            <Input
              size="sm"
              type="search"
              aria-label="Filter recordings"
              placeholder="Filter"
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              className="pl-6"
            />
          </div>
        )}
        {items.length === 0 ? (
          <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-1 px-4 text-center">
            <p className="text-ui-base font-medium">No recordings yet</p>
            <p className="text-ui-sm text-foreground-subtle">
              Drop recordings or folders on the window, or add them with +.
            </p>
          </div>
        ) : shown.length === 0 ? (
          <p className="px-2.5 py-2 text-ui-sm text-foreground-subtle">
            No recordings match “{filter.trim()}”.
          </p>
        ) : (
          <QueueList items={shown} />
        )}
      </div>
    </aside>
  );
}

/** The add button and the queue menu in the sidebar header. */
function QueueActions() {
  const hasItems = useQueueStore((state) => state.items.length > 0);
  const hasFinished = useQueueStore((state) => state.items.some((item) => isFinished(item.status)));
  const queue = useQueueStore.getState;
  return (
    <>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-md"
            aria-label="Add recordings"
            className="text-foreground-subtle hover:text-foreground"
            onClick={() => void browseFiles()}
          >
            <Plus />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          Add recordings
          <Kbd>{SHORTCUTS.addFiles.keys.join("+")}</Kbd>
        </TooltipContent>
      </Tooltip>
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="icon-md"
            aria-label="Queue menu"
            className="text-foreground-subtle hover:text-foreground"
          >
            <Ellipsis />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="min-w-52">
          <DropdownMenuItem onSelect={() => void browseFolder()}>
            <FolderPlus aria-hidden />
            Add folder…
            <DropdownMenuShortcut>{SHORTCUTS.addFolder.keys.join("+")}</DropdownMenuShortcut>
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuLabel>Sort by</DropdownMenuLabel>
          <DropdownMenuItem disabled={!hasItems} onSelect={() => queue().sortBy("name")}>
            Name
          </DropdownMenuItem>
          <DropdownMenuItem disabled={!hasItems} onSelect={() => queue().sortBy("added")}>
            Date added
          </DropdownMenuItem>
          <DropdownMenuItem disabled={!hasItems} onSelect={() => queue().sortBy("status")}>
            Status
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem disabled={!hasFinished} onSelect={() => queue().removeFinished()}>
            Clear finished
          </DropdownMenuItem>
          <DropdownMenuItem
            variant="destructive"
            disabled={!hasItems}
            onSelect={() => queue().removeAll()}
          >
            Remove all
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </>
  );
}
