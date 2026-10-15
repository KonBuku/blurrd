/**
 * Keeps the backend's override list in step with the edit store (plan
 * 5.9): every change sends the full list with the revision it was based
 * on, debounced, one request at a time. A `StaleRevision` answer means
 * the project changed elsewhere (a re-analysis, another window); the
 * caller then reloads the stored list, which replaces the local one.
 */
import type { AppError } from "@/bindings/AppError";
import type { Override } from "@/bindings/Override";
import type { OverridesSaved } from "@/bindings/OverridesSaved";
import { toAppError } from "@/ipc/errors";
import { useEditsStore } from "@/store/edits";

/** Debounce between the last edit and the save, in milliseconds (plan 5.9). */
export const SAVE_DEBOUNCE_MS = 150;

/** What the sync needs from the backend and the rest of the app. */
export interface SyncHandlers {
  /** Stores the full list (`set_overrides`). */
  readonly save: (
    projectId: string,
    baseRev: number,
    overrides: readonly Override[],
  ) => Promise<OverridesSaved>;
  /** The backend stored `overrides` (update cached project data). */
  readonly onSaved: (saved: OverridesSaved, overrides: readonly Override[]) => void;
  /**
   * The backend rejected a stale revision: read the stored edits again
   * and pass them to {@link OverrideSync.adopt}.
   */
  readonly onStale: () => Promise<void>;
  /** A save failed for another reason; the edits stay unsaved. */
  readonly onError: (error: AppError) => void;
}

/** Sends the edits of one project; create one per opened project and start it. */
export class OverrideSync {
  private readonly projectId: string;
  private readonly handlers: SyncHandlers;
  private readonly debounceMs: number;
  /** The list the backend has stored (as far as this sync knows). */
  private stored: readonly Override[] | null = null;
  private timer: ReturnType<typeof setTimeout> | null = null;
  /** The running save, if any. */
  private running: Promise<void> | null = null;
  /** Stops listening to the store; `null` while not started. */
  private unsubscribe: (() => void) | null = null;

  constructor(projectId: string, handlers: SyncHandlers, debounceMs = SAVE_DEBOUNCE_MS) {
    this.projectId = projectId;
    this.handlers = handlers;
    this.debounceMs = debounceMs;
  }

  /** Starts saving the store's changes (again after {@link dispose}). */
  start(): void {
    if (this.unsubscribe) return;
    this.unsubscribe = useEditsStore.subscribe((state, previous) => {
      if (state.overrides !== previous.overrides && this.owns()) this.changed();
    });
  }

  /**
   * Loads the stored edits of the project at `revision` into the store
   * (forgetting the undo history) and takes them as the saved state.
   */
  adopt(revision: number, overrides: readonly Override[]): void {
    this.cancelTimer();
    this.stored = overrides;
    useEditsStore.getState().load(this.projectId, revision, overrides);
  }

  /** Whether the store holds this sync's project. */
  owns(): boolean {
    return useEditsStore.getState().projectId === this.projectId;
  }

  /** Whether the stored edits were loaded ({@link adopt}). */
  loaded(): boolean {
    return this.stored !== null;
  }

  /** Whether a save is waiting or running. */
  busy(): boolean {
    return this.timer !== null || this.running !== null;
  }

  /**
   * Saves now instead of after the debounce (also retries a failed save);
   * resolves once the store's edits are stored or a save failed.
   */
  async flush(): Promise<void> {
    this.cancelTimer();
    if (this.running) {
      // One request at a time: wait for the running save, then look again.
      await this.running;
      return this.flush();
    }
    if (!this.owns() || this.stored === null) return;
    const { overrides } = useEditsStore.getState();
    if (overrides === this.stored) {
      useEditsStore.getState().setStatus("saved");
      return;
    }
    this.running = this.send(overrides);
    await this.running;
  }

  /** Stops listening; a waiting save is sent at once. */
  dispose(): void {
    this.unsubscribe?.();
    this.unsubscribe = null;
    if (this.timer !== null) void this.flush();
  }

  /** The store's list changed (an edit, undo or redo). */
  private changed(): void {
    if (this.stored === null) return;
    const store = useEditsStore.getState();
    this.cancelTimer();
    if (store.overrides === this.stored && this.running === null) {
      // Undo went back to what is stored: nothing to send.
      store.setStatus("saved");
      return;
    }
    store.setStatus("pending");
    this.timer = setTimeout(() => {
      this.timer = null;
      void this.flush();
    }, this.debounceMs);
  }

  private cancelTimer(): void {
    if (this.timer === null) return;
    clearTimeout(this.timer);
    this.timer = null;
  }

  /** One save of `overrides`, then another one if the list changed meanwhile. */
  private async send(overrides: readonly Override[]): Promise<void> {
    const store = useEditsStore.getState();
    store.setStatus("saving");
    let stale = false;
    try {
      const saved = await this.handlers.save(this.projectId, store.baseRev, overrides);
      this.stored = overrides;
      if (this.owns()) useEditsStore.getState().confirm(saved.revision);
      this.handlers.onSaved(saved, overrides);
    } catch (reason) {
      const error = toAppError(reason);
      if (error.kind !== "StaleRevision") {
        this.running = null;
        if (this.owns()) useEditsStore.getState().setStatus("failed");
        this.handlers.onError(error);
        return;
      }
      stale = true;
    }
    this.running = null;
    if (stale) {
      await this.handlers.onStale();
      return;
    }
    if (!this.owns()) return;
    if (useEditsStore.getState().overrides !== this.stored) {
      if (this.timer === null) await this.flush();
    } else {
      useEditsStore.getState().setStatus("saved");
    }
  }
}
