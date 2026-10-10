import { create } from "zustand";

import type { AppError } from "@/bindings/AppError";
import type { MediaInfo } from "@/bindings/MediaInfo";
import { isProbed, type ProbeOutcome, type SessionEntry } from "@/ipc/commands";
import { baseName, looksLikeFile, pathKey } from "@/lib/media";
import { isFinished, statusRank, type QueueStatus } from "@/lib/queue-status";

/** One file in the queue. */
export interface QueueItem {
  /** Stable id: the normalised path (`pathKey`). */
  readonly id: string;
  /** Path as the backend reported it. */
  readonly path: string;
  /** File name shown in the list. */
  readonly name: string;
  /** Media facts; `null` while the file is being read. */
  readonly media: MediaInfo | null;
  readonly status: QueueStatus;
  /** The exported file, once there is one. */
  readonly output: string | null;
  /** Order of addition, for "Sort by date added". */
  readonly added: number;
}

/** How a click or key changes the selection. */
export type SelectMode =
  /** Select only this item. */
  | "replace"
  /** Add or remove this item (Ctrl). */
  | "toggle"
  /** Select from the anchor to this item (Shift). */
  | "range";

/** Orders of "Sort by". */
export type SortKey = "name" | "added" | "status";

/** Outcome of resolving a batch of read paths. */
export interface ProbeResolution {
  /** Paths that were added. */
  readonly added: readonly string[];
  /** Outcomes that are not usable videos. */
  readonly rejected: readonly Extract<ProbeOutcome, { error: unknown }>[];
  /** Videos that were already in the queue. */
  readonly duplicates: readonly string[];
}

/** The queue: files, selection and the list filter. */
export interface QueueState {
  readonly items: readonly QueueItem[];
  /** Ids of the selected items. */
  readonly selected: ReadonlySet<string>;
  /** Item that range selection (Shift) starts from. */
  readonly anchor: string | null;
  /** Item the keyboard is on (the last one clicked or moved to). */
  readonly focused: string | null;
  /** Text filter of the list (case-insensitive file name match). */
  readonly filter: string;
  /**
   * Adds a placeholder in the "Reading…" state for each path that is not
   * in the queue yet and returns those paths.
   */
  readonly addPending: (paths: readonly string[]) => string[];
  /**
   * Replaces the placeholders of `pending` with the videos the probe
   * found, at the position of the first placeholder, and reports what
   * could not be added.
   */
  readonly resolvePending: (
    pending: readonly string[],
    outcomes: readonly ProbeOutcome[],
  ) => ProbeResolution;
  /** Removes the placeholders of `pending` (the probe failed as a whole). */
  readonly dropPending: (pending: readonly string[]) => void;
  /**
   * Puts the files of a saved session in front of the queue, with their
   * saved status and no media facts yet; files already queued are
   * skipped. Returns the paths that were added.
   */
  readonly restore: (entries: readonly SessionEntry[]) => string[];
  /**
   * Fills in the media facts of restored files from a fresh probe. A file
   * that can no longer be read is marked failed with the reason.
   */
  readonly refresh: (outcomes: readonly ProbeOutcome[]) => void;
  /**
   * Changes the status of an item (a job started, progressed or ended),
   * and its exported file when `output` is given.
   */
  readonly setStatus: (id: string, status: QueueStatus, output?: string) => void;
  readonly remove: (ids: Iterable<string>) => void;
  readonly removeSelected: () => void;
  /** Removes every item in an end state (done, failed, canceled, …). */
  readonly removeFinished: () => void;
  readonly removeAll: () => void;
  readonly sortBy: (key: SortKey) => void;
  readonly select: (id: string, mode: SelectMode) => void;
  /** Selects every item the filter shows. */
  readonly selectAll: () => void;
  /** Changes the filter; items it hides leave the selection. */
  readonly setFilter: (filter: string) => void;
}

/** Counter for {@link QueueItem.added}. */
let addedCounter = 0;

/** Case-insensitive natural order of file names ("clip2" before "clip10"). */
const NAME_ORDER = new Intl.Collator("en", { numeric: true, sensitivity: "base" });

function pendingItem(path: string): QueueItem {
  addedCounter += 1;
  return {
    id: pathKey(path),
    path,
    name: baseName(path),
    media: null,
    status: { kind: "probing", folder: !looksLikeFile(path) },
    output: null,
    added: addedCounter,
  };
}

/** Status of a restored file whose probe failed. */
function unreadable(error: AppError): QueueStatus {
  const reason = error.kind === "NotFound" ? "File not found" : "File can't be read";
  return { kind: "failed", reason, detail: error.message };
}

/** Items the filter shows, in list order. */
export function visibleItems(items: readonly QueueItem[], filter: string): readonly QueueItem[] {
  const needle = filter.trim().toLowerCase();
  if (needle === "") return items;
  return items.filter((item) => item.name.toLowerCase().includes(needle));
}

/** Removes ids that are no longer in `items` from the selection. */
function pruneSelection(state: QueueState, items: readonly QueueItem[]): Partial<QueueState> {
  const ids = new Set(items.map((item) => item.id));
  const selected = new Set([...state.selected].filter((id) => ids.has(id)));
  const keep = (id: string | null) => (id !== null && ids.has(id) ? id : null);
  return { items, selected, anchor: keep(state.anchor), focused: keep(state.focused) };
}

/**
 * The queue store. Selection and filter are view state; the items are
 * what a later session restore writes and reads.
 */
export const useQueueStore = create<QueueState>()((set, get) => ({
  items: [],
  selected: new Set(),
  anchor: null,
  focused: null,
  filter: "",

  addPending: (paths) => {
    const known = new Set(get().items.map((item) => item.id));
    const fresh: QueueItem[] = [];
    for (const path of paths) {
      const item = pendingItem(path);
      if (known.has(item.id)) continue;
      known.add(item.id);
      fresh.push(item);
    }
    if (fresh.length > 0) set((state) => ({ items: [...state.items, ...fresh] }));
    return fresh.map((item) => item.path);
  },

  resolvePending: (pending, outcomes) => {
    const pendingIds = new Set(pending.map(pathKey));
    const { items } = get();
    const insertAt = items.findIndex((item) => pendingIds.has(item.id));
    const kept = items.filter((item) => !pendingIds.has(item.id));
    const known = new Set(kept.map((item) => item.id));
    const order = new Map(
      items.filter((item) => pendingIds.has(item.id)).map((item) => [item.id, item.added]),
    );

    const fresh: QueueItem[] = [];
    const added: string[] = [];
    const duplicates: string[] = [];
    const rejected: Extract<ProbeOutcome, { error: unknown }>[] = [];
    for (const outcome of outcomes) {
      if (!isProbed(outcome)) {
        rejected.push(outcome);
        continue;
      }
      const id = pathKey(outcome.path);
      if (known.has(id)) {
        duplicates.push(outcome.path);
        continue;
      }
      known.add(id);
      addedCounter += 1;
      fresh.push({
        id,
        path: outcome.path,
        name: baseName(outcome.path),
        media: outcome.media,
        status: { kind: "queued" },
        output: null,
        added: order.get(id) ?? addedCounter,
      });
      added.push(outcome.path);
    }

    const start = insertAt === -1 ? kept.length : Math.min(insertAt, kept.length);
    const next = [...kept.slice(0, start), ...fresh, ...kept.slice(start)];
    set((state) => pruneSelection(state, next));
    return { added, rejected, duplicates };
  },

  dropPending: (pending) => {
    const pendingIds = new Set(pending.map(pathKey));
    set((state) =>
      pruneSelection(
        state,
        state.items.filter((item) => !pendingIds.has(item.id)),
      ),
    );
  },

  restore: (entries) => {
    const known = new Set(get().items.map((item) => item.id));
    const restored: QueueItem[] = [];
    for (const entry of entries) {
      const id = pathKey(entry.path);
      if (known.has(id)) continue;
      known.add(id);
      addedCounter += 1;
      restored.push({
        id,
        path: entry.path,
        name: baseName(entry.path),
        media: null,
        status: entry.status,
        output: entry.output,
        added: addedCounter,
      });
    }
    if (restored.length > 0) set((state) => ({ items: [...restored, ...state.items] }));
    return restored.map((item) => item.path);
  },

  refresh: (outcomes) => {
    const byId = new Map(outcomes.map((outcome) => [pathKey(outcome.path), outcome]));
    set((state) => ({
      items: state.items.map((item) => {
        const outcome = byId.get(item.id);
        if (!outcome) return item;
        return isProbed(outcome)
          ? { ...item, media: outcome.media }
          : { ...item, status: unreadable(outcome.error) };
      }),
    }));
  },

  remove: (ids) => {
    const gone = new Set(ids);
    set((state) =>
      pruneSelection(
        state,
        state.items.filter((item) => !gone.has(item.id)),
      ),
    );
  },

  setStatus: (id, status, output) =>
    set((state) => ({
      items: state.items.map((item) =>
        item.id === id ? { ...item, status, output: output ?? item.output } : item,
      ),
    })),

  removeSelected: () => get().remove(get().selected),

  removeFinished: () =>
    set((state) =>
      pruneSelection(
        state,
        state.items.filter((item) => !isFinished(item.status)),
      ),
    ),

  removeAll: () => set({ items: [], selected: new Set(), anchor: null, focused: null }),

  sortBy: (key) =>
    set((state) => {
      const items = [...state.items];
      switch (key) {
        case "name":
          items.sort((a, b) => NAME_ORDER.compare(a.name, b.name));
          break;
        case "added":
          items.sort((a, b) => a.added - b.added);
          break;
        case "status":
          items.sort((a, b) => statusRank(a.status) - statusRank(b.status));
          break;
      }
      return { items };
    }),

  select: (id, mode) =>
    set((state) => {
      if (mode === "toggle") {
        const selected = new Set(state.selected);
        if (selected.has(id)) selected.delete(id);
        else selected.add(id);
        return { selected, anchor: id, focused: id };
      }
      if (mode === "range" && state.anchor !== null) {
        const visible = visibleItems(state.items, state.filter);
        const from = visible.findIndex((item) => item.id === state.anchor);
        const to = visible.findIndex((item) => item.id === id);
        if (from !== -1 && to !== -1) {
          const [lo, hi] = from < to ? [from, to] : [to, from];
          // The anchor stays where the range started.
          return {
            selected: new Set(visible.slice(lo, hi + 1).map((item) => item.id)),
            focused: id,
          };
        }
      }
      return { selected: new Set([id]), anchor: id, focused: id };
    }),

  selectAll: () =>
    set((state) => ({
      selected: new Set(visibleItems(state.items, state.filter).map((item) => item.id)),
    })),

  setFilter: (filter) =>
    set((state) => {
      const shown = new Set(visibleItems(state.items, filter).map((item) => item.id));
      return { filter, selected: new Set([...state.selected].filter((id) => shown.has(id))) };
    }),
}));

/**
 * The queue item the preview and the inspector show: the focused item when
 * it is selected, otherwise the first selected one; `null` when nothing
 * is selected.
 */
export function shownItemId(queue: QueueState): string | null {
  const { focused, selected } = queue;
  if (focused !== null && selected.has(focused)) return focused;
  return selected.values().next().value ?? null;
}
