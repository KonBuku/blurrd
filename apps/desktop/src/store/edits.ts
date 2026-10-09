import { temporal } from "zundo";
import { create } from "zustand";

import type { Override } from "@/bindings/Override";

/**
 * Whether the edits on screen are stored in the project: `saved`, waiting
 * for the debounce (`pending`), on their way (`saving`) or not stored
 * because the last save failed (`failed`).
 */
export type SaveStatus = "saved" | "pending" | "saving" | "failed";

/** The user edits of the shown project (plan 5.9 `project` slice). */
export interface EditsState {
  /** The project the edits belong to; `null` before one is loaded. */
  readonly projectId: string | null;
  /** The override list as the user sees it; every version is an undo step. */
  readonly overrides: readonly Override[];
  /** Project revision the backend confirmed last (`base_rev` of the next save). */
  readonly baseRev: number;
  readonly status: SaveStatus;
  /**
   * Replaces the edits with a project's stored list at `revision` and
   * forgets the undo history (opening a project, reloading after a stale
   * revision or a re-analysis).
   */
  readonly load: (projectId: string, revision: number, overrides: readonly Override[]) => void;
  /** Applies a new list made by one user action (one undo step). */
  readonly commit: (overrides: readonly Override[]) => void;
  /** Records that the backend stored the edits as `revision`. */
  readonly confirm: (revision: number) => void;
  readonly setStatus: (status: SaveStatus) => void;
}

/** Undo steps kept per project. */
export const HISTORY_LIMIT = 500;

/**
 * Owns the override list of the shown project (plan 5.9): edits change it
 * locally at once, zundo keeps every version for undo and redo, and
 * `OverrideSync` sends the whole list with `base_rev` to the backend.
 * Only the list is part of the history; revision and save status are not.
 */
export const useEditsStore = create<EditsState>()(
  temporal(
    (set) => ({
      projectId: null,
      overrides: [],
      baseRev: 0,
      status: "saved",
      load: (projectId, revision, overrides) => {
        const history = useEditsStore.temporal.getState();
        history.pause();
        set({ projectId, overrides, baseRev: revision, status: "saved" });
        history.resume();
        history.clear();
      },
      commit: (overrides) => set({ overrides }),
      confirm: (baseRev) => set({ baseRev }),
      setStatus: (status) => set({ status }),
    }),
    {
      partialize: (state) => ({ overrides: state.overrides }),
      equality: (past, current) => past.overrides === current.overrides,
      limit: HISTORY_LIMIT,
    },
  ),
);

/** Undoes the last edit of the shown project. */
export function undoEdit(): void {
  useEditsStore.temporal.getState().undo();
}

/** Redoes the last undone edit. */
export function redoEdit(): void {
  useEditsStore.temporal.getState().redo();
}
