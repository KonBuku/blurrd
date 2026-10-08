import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import { INSPECTOR, SIDEBAR, type LayoutPreferences } from "@/lib/layout";

/** Tabs of the inspector frame. */
export type InspectorTab = "blur" | "export" | "verify";

/** State of the window shell. */
export interface UiState extends LayoutPreferences {
  /** Whether the inspector sheet is open (narrow windows only). */
  readonly inspectorSheetOpen: boolean;
  /** The selected inspector tab. */
  readonly inspectorTab: InspectorTab;
  readonly toggleSidebar: () => void;
  readonly setSidebarWidth: (width: number) => void;
  /**
   * Shows or hides the inspector: the sheet when the window is narrow
   * (`asSheet`), otherwise the frame.
   */
  readonly toggleInspector: (asSheet: boolean) => void;
  readonly setInspectorSheetOpen: (open: boolean) => void;
  readonly setInspectorWidth: (width: number) => void;
  readonly setInspectorTab: (tab: InspectorTab) => void;
}

/** The parts of {@link UiState} that survive a restart. */
type PersistedUi = Pick<
  UiState,
  "sidebarOpen" | "sidebarWidth" | "inspectorOpen" | "inspectorWidth" | "inspectorTab"
>;

/**
 * Shell state: panel visibility, preferred widths and the selected
 * inspector tab. Layout preferences are kept in the WebView's local
 * storage (a per-install convenience, not a setting); widths are stored as
 * the user left them and clamped to the window by `resolveLayout`.
 */
export const useUiStore = create<UiState>()(
  persist(
    (set) => ({
      sidebarOpen: true,
      sidebarWidth: SIDEBAR.default,
      inspectorOpen: true,
      inspectorWidth: INSPECTOR.default,
      inspectorSheetOpen: false,
      inspectorTab: "blur",
      toggleSidebar: () => set((state) => ({ sidebarOpen: !state.sidebarOpen })),
      setSidebarWidth: (sidebarWidth) => set({ sidebarWidth }),
      toggleInspector: (asSheet) =>
        set((state) =>
          asSheet
            ? { inspectorSheetOpen: !state.inspectorSheetOpen }
            : { inspectorOpen: !state.inspectorOpen },
        ),
      setInspectorSheetOpen: (inspectorSheetOpen) => set({ inspectorSheetOpen }),
      setInspectorWidth: (inspectorWidth) => set({ inspectorWidth }),
      setInspectorTab: (inspectorTab) => set({ inspectorTab }),
    }),
    {
      name: "blurrd.layout",
      version: 1,
      storage: createJSONStorage(() => localStorage),
      partialize: (state): PersistedUi => ({
        sidebarOpen: state.sidebarOpen,
        sidebarWidth: state.sidebarWidth,
        inspectorOpen: state.inspectorOpen,
        inspectorWidth: state.inspectorWidth,
        inspectorTab: state.inspectorTab,
      }),
    },
  ),
);
