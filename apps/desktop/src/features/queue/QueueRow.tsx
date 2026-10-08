import { Film, X } from "lucide-react";
import { memo, type ComponentProps, type MouseEvent } from "react";

import { Progress } from "@/components/ui/progress";
import { mediaSummary } from "@/lib/media";
import { statusProgress, statusText } from "@/lib/queue-status";
import { cn } from "@/lib/utils";
import type { QueueItem } from "@/store/queue";

import { StatusGlyph } from "./StatusGlyph";

export type QueueRowProps = Omit<
  ComponentProps<"div">,
  "children" | "onClick" | "onContextMenu" | "style"
> & {
  /** Vertical position inside a virtualized list, in pixels. */
  readonly offset?: number;
  readonly item: QueueItem;
  readonly selected: boolean;
  /** Whether the keyboard is on this row (shown while the list has focus). */
  readonly active: boolean;
  /** A click on the row; the list derives the selection mode from the event. */
  readonly onPick: (id: string, event: MouseEvent) => void;
  /** A right-click on the row, before the context menu opens. */
  readonly onContext: (id: string) => void;
  /** Removes the item; offered as a hover action (keyboard: Delete). */
  readonly onRemove: (id: string) => void;
};

/**
 * One file in the queue (design 8.1): thumbnail tile, file name, media
 * facts and the status line with its glyph and, while a job runs, an
 * inline progress bar. A listbox option; the remove action on hover is a
 * mouse shortcut for Delete and the context menu, so it is not a tab stop.
 * Memoized: while the list scrolls, rows that stay mounted do not render
 * again (the callbacks must be stable).
 */
export const QueueRow = memo(function QueueRow({
  item,
  selected,
  active,
  onPick,
  onContext,
  onRemove,
  offset,
  className,
  ...props
}: QueueRowProps) {
  const progress = statusProgress(item.status);
  const status = statusText(item.status);
  return (
    // The listbox owns focus and the keyboard (aria-activedescendant), so
    // its options take clicks but are not focusable themselves.
    // oxlint-disable-next-line jsx-a11y/click-events-have-key-events, jsx-a11y/interactive-supports-focus
    <div
      // Rows hold a thumbnail and three lines of text, which <option>
      // cannot render.
      // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
      role="option"
      aria-selected={selected}
      data-active={active || undefined}
      className={cn(
        "group/row relative flex gap-2 rounded-lg px-2 py-1.5 transition-colors hover:bg-hover",
        selected && "bg-selected hover:bg-selected",
        "group-focus-visible/list:data-active:outline-2 group-focus-visible/list:data-active:-outline-offset-2 group-focus-visible/list:data-active:outline-ring group-focus-visible/list:data-active:outline-solid",
        className,
      )}
      style={offset === undefined ? undefined : { transform: `translateY(${offset}px)` }}
      onClick={(event) => onPick(item.id, event)}
      onContextMenu={() => onContext(item.id)}
      {...props}
    >
      <div className="grid h-9 w-16 shrink-0 place-items-center rounded-md bg-surface text-foreground-subtlest">
        <Film aria-hidden className="size-4" />
      </div>
      <div className="flex min-w-0 flex-1 flex-col">
        <span className="truncate pr-5 text-ui-base/5">{item.name}</span>
        <span className="truncate text-ui-xs/4 text-foreground-subtle tabular-nums">
          {item.media ? mediaSummary(item.media) : " "}
        </span>
        <span className="flex min-w-0 items-center gap-1.5 text-ui-sm/4 tabular-nums">
          <StatusGlyph status={item.status} />
          <span className="truncate">{status}</span>
        </span>
        {progress !== undefined && (
          <Progress
            aria-label={`${item.name}: ${status}`}
            value={progress === null ? null : progress * 100}
            className="mt-1"
          />
        )}
      </div>
      <span
        aria-hidden
        title="Remove from queue"
        className="absolute top-1 right-1 hidden size-5 place-items-center rounded-sm text-foreground-subtle group-hover/row:grid hover:bg-hover hover:text-foreground"
        onPointerDown={(event) => event.stopPropagation()}
        onClick={(event) => {
          event.stopPropagation();
          onRemove(item.id);
        }}
      >
        <X className="size-3" />
      </span>
    </div>
  );
});
