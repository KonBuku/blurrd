import { create } from "zustand";
import { createJSONStorage, persist } from "zustand/middleware";

import { INSPECTOR, SIDEBAR, type LayoutPreferences } from "@/lib/layout";

/** Tabs of the inspector frame. */
export type InspectorTab = "blur" | "export" | "verify";

/** Pages of the settings view. */
export type SettingsSection = "general" | "appearance" | "shortcuts" | "about";

/** What the content area shows: the workspace frames or the settings. */
export type View = "workspace" | "settings";

/** State of the window shell. */
export interface UiState extends LayoutPreferences {
  /** Whether the inspector sheet is open (narrow windows only). */
  readonly inspectorSheetOpen: boolean;
  /** The selected inspector tab. */
  readonly inspectorTab: InspectorTab;
  /** Whether the settings replace the preview and inspector. */
  readonly view: View;
  /** The settings page shown. */
  readonly settingsSection: SettingsSection;
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
  /** Shows the settings, on `section` or the last page shown. */
  readonly openSettings: (section?: SettingsSection) => void;
  /** Returns to the workspace. */
  readonly closeSettings: () => void;
}

/** The parts of {@link UiState} that survive a restart. */
type PersistedUi = Pick<
  UiState,
  "sidebarOpen" | "sidebarWidth" | "inspectorOpen" | "inspectorWidth" | "inspectorTab"
>;

/**
 * Shell state: panel visibility, preferred widths, the selected
 * inspector tab and whether the settings are open. Layout preferences are kept in the WebView's local
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
      view: "workspace",
      settingsSection: "appearance",
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
      openSettings: (section) =>
        set((state) => ({
          view: "settings",
          settingsSection: section ?? state.settingsSection,
          inspectorSheetOpen: false,
        })),
      closeSettings: () => set({ view: "workspace" }),
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
