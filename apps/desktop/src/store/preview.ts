import { create } from "zustand";

import type { PreviewMode } from "@/features/preview/gl/blur-renderer";
import type { Point, Zoom } from "@/lib/stage";

/** How the preview shows the frame and where the user looks. */
export interface PreviewState {
  /** Before (source), After (blurred) or Split. */
  readonly mode: PreviewMode;
  /** Position of the split divider as a fraction of the frame width. */
  readonly split: number;
  /** Whether the box overlay is shown. */
  readonly showBoxes: boolean;
  /**
   * Whether paused frames show the export's own blur of the HUD regions
   * (backend patches) instead of the WebGL approximation.
   */
  readonly exact: boolean;
  readonly zoom: Zoom;
  /** Source point at the stage centre; `null` = the frame centre. */
  readonly center: Point | null;
  /** Key of the selected overlay item (track, edit or leak), if any. */
  readonly selected: string | null;
  /**
   * The manual box tool while it is active: the frames (first and last,
   * inclusive) the next drawn box covers.
   */
  readonly tool: { readonly range: readonly [number, number] } | null;
  readonly setMode: (mode: PreviewMode) => void;
  readonly setSplit: (split: number) => void;
  readonly toggleBoxes: () => void;
  readonly toggleExact: () => void;
  /** Sets the zoom, and the point at the stage centre when given. */
  readonly setZoom: (zoom: Zoom, center?: Point | null) => void;
  readonly setCenter: (center: Point) => void;
  readonly select: (key: string | null) => void;
  /** Starts the manual box tool for `range` (shows the overlay), or changes its range. */
  readonly setTool: (range: readonly [number, number]) => void;
  /** Leaves the manual box tool. */
  readonly endTool: () => void;
  /** Leaves the tool and clears the selection (a new project). */
  readonly resetEditing: () => void;
}

/** View state of the preview stage (not persisted: every session starts at Fit). */
export const usePreviewStore = create<PreviewState>()((set) => ({
  mode: "after",
  split: 0.5,
  showBoxes: true,
  exact: false,
  zoom: "fit",
  center: null,
  selected: null,
  tool: null,
  setMode: (mode) => set({ mode }),
  setSplit: (split) => set({ split: Math.min(Math.max(split, 0), 1) }),
  toggleBoxes: () => set((state) => ({ showBoxes: !state.showBoxes })),
  toggleExact: () => set((state) => ({ exact: !state.exact })),
  setZoom: (zoom, center) =>
    set((state) => ({
      zoom,
      center: zoom === "fit" ? null : center === undefined ? state.center : center,
    })),
  setCenter: (center) => set({ center }),
  select: (selected) => set({ selected }),
  setTool: (range) => set({ tool: { range }, showBoxes: true }),
  endTool: () => set({ tool: null }),
  resetEditing: () => set({ tool: null, selected: null }),
}));
