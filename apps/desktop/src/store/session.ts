/**
 * Session restore (acceptance M23): the queue is saved to the backend's
 * `session.json` whenever it changes and put back on the next start when
 * Settings › General › "Restore the queue" is on. With the setting off, the
 * saved queue is cleared, so no paths are kept.
 */
import { getSession, probeMedia, saveSession, type SessionEntry } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { logWarning } from "@/lib/log";
import { parseRestingStatus, restingStatus } from "@/lib/queue-status";

import { useQueueStore, type QueueItem } from "./queue";
import { useSettingsStore } from "./settings";

/** Changes are written this long after the last one, in milliseconds. */
export const SAVE_DELAY_MS = 300;

/** The saved form of the queue: placeholders still being read are left out. */
export function sessionEntries(items: readonly QueueItem[]): SessionEntry[] {
  return items
    .filter((item) => item.status.kind !== "probing")
    .map((item) => ({ path: item.path, status: restingStatus(item.status), output: item.output }));
}

/** Reads saved entries from untrusted JSON, dropping malformed ones. */
export function parseSession(value: unknown): SessionEntry[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((raw: unknown) => {
    if (typeof raw !== "object" || raw === null) return [];
    const entry = raw as Record<string, unknown>;
    if (typeof entry.path !== "string" || entry.path === "") return [];
    return [
      {
        path: entry.path,
        status: parseRestingStatus(entry.status),
        output: typeof entry.output === "string" ? entry.output : null,
      },
    ];
  });
}

/** Puts the saved queue back and reads the restored files again. */
async function restore(): Promise<void> {
  let entries: SessionEntry[];
  try {
    entries = parseSession(await getSession());
  } catch (reason) {
    logWarning("reading the saved queue failed", toAppError(reason).message);
    return;
  }
  const restored = useQueueStore.getState().restore(entries);
  if (restored.length === 0) return;
  try {
    useQueueStore.getState().refresh(await probeMedia(restored));
  } catch (reason) {
    // The items stay with their saved status; only the media facts are missing.
    logWarning("reading the restored files failed", toAppError(reason).message);
  }
}

/** Writes the queue (or an empty one when restoring is off). */
async function save(): Promise<void> {
  const keep = useSettingsStore.getState().general.restore_queue;
  const entries = keep ? sessionEntries(useQueueStore.getState().items) : [];
  try {
    await saveSession(entries);
  } catch (reason) {
    logWarning("saving the queue failed", toAppError(reason).message);
  }
}

/**
 * Restores the saved queue (when the setting allows it) and then saves
 * every change, debounced. Call once after the settings have loaded;
 * saving starts only after the restore, so a slow start never overwrites
 * the saved queue with an empty one. Returns a function that stops saving.
 */
export async function startSession(): Promise<() => void> {
  if (useSettingsStore.getState().general.restore_queue) await restore();
  else await save();

  let timer: ReturnType<typeof setTimeout> | undefined;
  const schedule = () => {
    clearTimeout(timer);
    timer = setTimeout(() => void save(), SAVE_DELAY_MS);
  };
  const stopQueue = useQueueStore.subscribe((state, previous) => {
    if (state.items !== previous.items) schedule();
  });
  const stopSettings = useSettingsStore.subscribe((state, previous) => {
    if (state.general.restore_queue !== previous.general.restore_queue) schedule();
  });
  return () => {
    clearTimeout(timer);
    stopQueue();
    stopSettings();
  };
}
