import { useVirtualizer } from "@tanstack/react-virtual";
import { CircleAlert, Copy, FolderOpen, Trash2 } from "lucide-react";
import { useCallback, useEffect, useRef, type KeyboardEvent, type MouseEvent } from "react";
import { toast } from "sonner";

import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu";
import { DropdownMenuShortcut } from "@/components/ui/dropdown-menu";
import { showFailureDetails } from "@/features/jobs/FailureDetailsDialog";
import { showInFolder } from "@/features/jobs/batch-end";
import { logWarning } from "@/lib/log";
import { formatCount } from "@/lib/media";
import { SHORTCUTS } from "@/lib/shortcuts";
import { useQueueStore, type QueueItem, type SelectMode } from "@/store/queue";

import { QueueRow } from "./QueueRow";

/** Estimated row height before a row is measured (64 px row + 2 px gap). */
const ESTIMATED_ROW = 66;
/** Rows rendered beyond the visible ones in each direction. */
const OVERSCAN = 6;

/** DOM id of a row, for `aria-activedescendant`. */
function optionId(item: QueueItem): string {
  return `queue-option-${item.added}`;
}

/** The selection mode a click with these modifiers asks for. */
function clickMode(event: MouseEvent): SelectMode {
  if (event.shiftKey) return "range";
  if (event.ctrlKey || event.metaKey) return "toggle";
  return "replace";
}

export interface QueueListProps {
  /** The rows the filter shows, in list order. */
  readonly items: readonly QueueItem[];
}

/**
 * The queue as a virtualized multi-select listbox (users drop whole
 * recording folders): only the visible rows are in the DOM. One tab stop;
 * arrows, Home/End and Page keys move (Shift extends, Ctrl+A selects all),
 * Delete removes, the context menu offers the same actions.
 */
export function QueueList({ items }: QueueListProps) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const selected = useQueueStore((state) => state.selected);
  const focused = useQueueStore((state) => state.focused);
  const select = useQueueStore((state) => state.select);

  // The virtualizer returns fresh measurements on every render by design.
  // oxlint-disable-next-line react/incompatible-library
  const virtualizer = useVirtualizer({
    count: items.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => ESTIMATED_ROW,
    overscan: OVERSCAN,
    gap: 2,
    getItemKey: (index) => items[index]?.id ?? index,
    // Let React batch the re-render of a scroll step with the frame
    // instead of rendering synchronously inside every scroll event.
    useFlushSync: false,
  });

  const focusedIndex = items.findIndex((item) => item.id === focused);
  const focusedItem = items[focusedIndex];

  // Keep the keyboard row rendered and in view.
  useEffect(() => {
    if (focusedIndex !== -1) virtualizer.scrollToIndex(focusedIndex, { align: "auto" });
  }, [focusedIndex, virtualizer]);

  /** Moves the keyboard row to `index` (clamped), extending with Shift. */
  const moveTo = (index: number, mode: "replace" | "range") => {
    const target = items[Math.min(Math.max(index, 0), items.length - 1)];
    if (target) select(target.id, mode);
  };

  const onKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    if (items.length === 0) return;
    const page = Math.max(1, Math.floor((scrollRef.current?.clientHeight ?? 0) / ESTIMATED_ROW));
    const from = focusedIndex;
    const mode = event.shiftKey ? "range" : "replace";
    const ctrl = event.ctrlKey || event.metaKey;
    switch (event.key) {
      case "ArrowDown":
        moveTo(from + 1, mode);
        break;
      case "ArrowUp":
        moveTo(from === -1 ? items.length - 1 : from - 1, mode);
        break;
      case "PageDown":
        moveTo(from + page, mode);
        break;
      case "PageUp":
        moveTo(from - page, mode);
        break;
      case "Home":
        moveTo(0, mode);
        break;
      case "End":
        moveTo(items.length - 1, mode);
        break;
      case " ":
        if (focusedItem) select(focusedItem.id, ctrl ? "toggle" : "replace");
        break;
      case "a":
      case "A":
        if (!ctrl) return;
        useQueueStore.getState().selectAll();
        break;
      case SHORTCUTS.removeFromQueue.key:
        if (selected.size === 0) return;
        useQueueStore.getState().removeSelected();
        break;
      default:
        return;
    }
    event.preventDefault();
  };

  // Stable row callbacks, so memoized rows skip renders while scrolling.
  const onPick = useCallback(
    (id: string, event: MouseEvent) => select(id, clickMode(event)),
    [select],
  );
  const onContext = useCallback(
    (id: string) => {
      // Right-clicking outside the selection acts on that row alone.
      if (!useQueueStore.getState().selected.has(id)) select(id, "replace");
    },
    [select],
  );
  const onRemove = useCallback((id: string) => useQueueStore.getState().remove([id]), []);

  const count = selected.size;
  // Actions on one file: the right-clicked row (it is selected alone).
  const single = count === 1 ? items.find((item) => selected.has(item.id)) : undefined;
  const output = single?.output ?? null;
  const failure = single?.status.kind === "failed" ? single.status : null;

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div
          ref={scrollRef}
          // A virtualized list of rich rows (WAI-ARIA listbox pattern);
          // <select> cannot render or virtualize them.
          // oxlint-disable-next-line jsx-a11y/prefer-tag-over-role
          role="listbox"
          aria-label="Recordings"
          aria-multiselectable
          aria-activedescendant={focusedItem ? optionId(focusedItem) : undefined}
          tabIndex={0}
          onKeyDown={onKeyDown}
          className="group/list min-h-0 flex-1 overflow-y-auto pr-1 [scrollbar-gutter:stable] focus-visible:-outline-offset-2"
        >
          <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
            {virtualizer.getVirtualItems().map((row) => {
              const item = items[row.index];
              if (!item) return null;
              return (
                <QueueRow
                  key={row.key}
                  id={optionId(item)}
                  ref={virtualizer.measureElement}
                  data-index={row.index}
                  aria-setsize={items.length}
                  aria-posinset={row.index + 1}
                  item={item}
                  selected={selected.has(item.id)}
                  active={item.id === focused}
                  onPick={onPick}
                  onContext={onContext}
                  onRemove={onRemove}
                  offset={row.start}
                  className="absolute top-0 left-0 w-full"
                />
              );
            })}
          </div>
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem disabled={count === 0} onSelect={() => void copyPaths()}>
          <Copy aria-hidden />
          {count > 1 ? `Copy ${formatCount(count)} paths` : "Copy path"}
        </ContextMenuItem>
        {output !== null && (
          <ContextMenuItem onSelect={() => void showInFolder(output)}>
            <FolderOpen aria-hidden />
            Show export in folder
          </ContextMenuItem>
        )}
        {single && failure && (
          <ContextMenuItem
            onSelect={() =>
              showFailureDetails({
                title: `${single.name} failed`,
                message: failure.reason,
                detail: failure.detail,
              })
            }
          >
            <CircleAlert aria-hidden />
            Show error details…
          </ContextMenuItem>
        )}
        <ContextMenuSeparator />
        <ContextMenuItem
          variant="destructive"
          disabled={count === 0}
          onSelect={() => useQueueStore.getState().removeSelected()}
        >
          <Trash2 aria-hidden />
          {count > 1 ? `Remove ${formatCount(count)} recordings` : "Remove from queue"}
          <DropdownMenuShortcut>{SHORTCUTS.removeFromQueue.keys.join("+")}</DropdownMenuShortcut>
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  );
}

/** Copies the paths of the selected items, one per line, in list order. */
async function copyPaths(): Promise<void> {
  const { items, selected } = useQueueStore.getState();
  const text = items
    .filter((item) => selected.has(item.id))
    .map((item) => item.path)
    .join("\r\n");
  try {
    await navigator.clipboard.writeText(text);
  } catch (reason) {
    logWarning("copying the paths failed", reason);
    toast.error("The paths could not be copied");
  }
}
