import {
  FilePlus,
  FolderOpen,
  FolderPlus,
  Info,
  Keyboard,
  ListX,
  Palette,
  PanelLeft,
  PanelRight,
  ScrollText,
  Settings,
  SlidersHorizontal,
} from "lucide-react";
import { useMemo } from "react";

import type { Theme } from "@/bindings/Theme";
import { RegionIcon } from "@/components/ui/region-chip";
import { useAboutStore } from "@/features/about/store";
import { browseFiles, browseFolder } from "@/features/import/import";
import { showInFolder } from "@/features/jobs/batch-end";
import { useJobsStore } from "@/features/jobs/jobs";
import { THEME_OPTIONS } from "@/features/settings/AppearanceSettings";
import { ALL_REGIONS, orderedRegions } from "@/lib/job-settings";
import { INSPECTOR_SHEET_BREAKPOINT } from "@/lib/layout";
import { baseName } from "@/lib/media";
import { isFinished } from "@/lib/queue-status";
import { REGIONS } from "@/lib/regions";
import { settingsOf, targetsOf, useJobSettingsStore } from "@/store/job-settings";
import { shownItemId, useQueueStore } from "@/store/queue";
import { useSettingsStore } from "@/store/settings";
import { useUiStore } from "@/store/ui";

import { useCommandSource, type PaletteCommand } from "./registry";

/** Extra search words of each theme. */
const THEME_KEYWORDS: Readonly<Record<Theme, readonly string[]>> = {
  System: ["automatic", "windows"],
  Light: ["light mode", "day"],
  Dark: ["dark mode", "night"],
};

/** Commands of the window: files, view, theme, settings. */
function useWindowCommands(): readonly PaletteCommand[] {
  const sidebarOpen = useUiStore((state) => state.sidebarOpen);
  const inspectorOpen = useUiStore((state) => state.inspectorOpen);
  const view = useUiStore((state) => state.view);
  const theme = useSettingsStore((state) => state.appearance.theme);
  const lastExport = useJobsStore((state) => state.recent[0]?.path ?? null);
  const hasFinished = useQueueStore((state) => state.items.some((item) => isFinished(item.status)));

  return useMemo(() => {
    const ui = () => useUiStore.getState();
    const commands: PaletteCommand[] = [
      {
        id: "file.add-files",
        group: "File",
        label: "Add recordings…",
        keywords: ["open", "import", "video"],
        icon: <FilePlus aria-hidden />,
        shortcut: "addFiles",
        run: () => void browseFiles(),
      },
      {
        id: "file.add-folder",
        group: "File",
        label: "Add a folder of recordings…",
        keywords: ["open", "import", "directory"],
        icon: <FolderPlus aria-hidden />,
        shortcut: "addFolder",
        run: () => void browseFolder(),
      },
    ];
    if (lastExport !== null) {
      commands.push({
        id: "file.reveal-export",
        group: "File",
        label: `Show ${baseName(lastExport)} in folder`,
        keywords: ["reveal", "output", "explorer", "last export"],
        icon: <FolderOpen aria-hidden />,
        run: () => void showInFolder(lastExport),
      });
    }
    if (hasFinished) {
      commands.push({
        id: "file.remove-finished",
        group: "File",
        label: "Remove finished files from the queue",
        keywords: ["clear", "done"],
        icon: <ListX aria-hidden />,
        run: () => useQueueStore.getState().removeFinished(),
      });
    }
    commands.push(
      {
        id: "view.toggle-queue",
        group: "View",
        label: sidebarOpen ? "Hide the queue" : "Show the queue",
        keywords: ["sidebar"],
        icon: <PanelLeft aria-hidden />,
        shortcut: "toggleQueue",
        run: () => ui().toggleSidebar(),
      },
      {
        id: "view.toggle-inspector",
        group: "View",
        label: inspectorOpen && view === "workspace" ? "Hide the inspector" : "Show the inspector",
        keywords: ["panel", "settings"],
        icon: <PanelRight aria-hidden />,
        shortcut: "toggleInspector",
        run: () => {
          const asSheet = window.innerWidth < INSPECTOR_SHEET_BREAKPOINT;
          if (ui().view !== "workspace") {
            ui().closeSettings();
            if (asSheet) ui().setInspectorSheetOpen(true);
            else if (!ui().inspectorOpen) ui().toggleInspector(false);
            return;
          }
          ui().toggleInspector(asSheet);
        },
      },
      {
        id: "view.settings",
        group: "View",
        label: view === "settings" ? "Close settings" : "Open settings",
        keywords: ["preferences", "options"],
        icon: <Settings aria-hidden />,
        shortcut: view === "settings" ? "closeOverlay" : "openSettings",
        run: () => (ui().view === "settings" ? ui().closeSettings() : ui().openSettings()),
      },
      {
        id: "view.settings-general",
        group: "View",
        label: "Settings: General",
        keywords: ["notifications", "session"],
        icon: <SlidersHorizontal aria-hidden />,
        run: () => ui().openSettings("general"),
      },
      {
        id: "view.settings-appearance",
        group: "View",
        label: "Settings: Appearance",
        keywords: ["interface size", "motion", "theme"],
        icon: <Palette aria-hidden />,
        run: () => ui().openSettings("appearance"),
      },
      ...THEME_OPTIONS.map((option): PaletteCommand => ({
        id: `theme.${option.value.toLowerCase()}`,
        group: "Theme",
        label: `Theme: ${option.label}`,
        keywords: ["appearance", "colour", "color", ...THEME_KEYWORDS[option.value]],
        icon: option.icon,
        hint: option.value === theme ? "Current" : undefined,
        run: () => useSettingsStore.getState().updateAppearance({ theme: option.value }),
      })),
      {
        id: "help.shortcuts",
        group: "Help",
        label: "Keyboard shortcuts",
        keywords: ["keys", "hotkeys", "settings"],
        icon: <Keyboard aria-hidden />,
        run: () => ui().openSettings("shortcuts"),
      },
      {
        id: "help.about",
        group: "Help",
        label: "About blurrd",
        keywords: ["version", "commit", "trademarks"],
        icon: <Info aria-hidden />,
        run: () => useAboutStore.getState().show("about"),
      },
      {
        id: "help.licenses",
        group: "Help",
        label: "Open-source licenses",
        keywords: ["licences", "third-party", "notices", "credits"],
        icon: <ScrollText aria-hidden />,
        run: () => useAboutStore.getState().show("licenses"),
      },
    );
    return commands;
  }, [sidebarOpen, inspectorOpen, view, theme, lastExport, hasFinished]);
}

/**
 * One command per region that switches its blur for the inspector's
 * targets: the selected files (or the defaults for new files), or every
 * file when "Apply to" is "All".
 */
function useRegionCommands(): readonly PaletteCommand[] {
  const shownId = useQueueStore(shownItemId);
  const enabled = useJobSettingsStore((state) => settingsOf(state, shownId).regions.enabled);

  return useMemo(
    () =>
      ALL_REGIONS.map((region): PaletteCommand => {
        const on = enabled.includes(region);
        const name = REGIONS[region].label;
        return {
          id: `blur.${region}`,
          group: "Blur",
          label: on ? `${name}: stop blurring names` : `${name}: blur names`,
          keywords: ["region", name, on ? "turn off" : "turn on"],
          icon: <RegionIcon region={region} />,
          hint: on ? "On" : "Off",
          run: () => {
            const settings = useJobSettingsStore.getState();
            const targets = targetsOf(useQueueStore.getState(), settings.scope);
            settings.updateSection(targets, "regions", (current) => {
              const next = new Set(current.enabled);
              if (on) next.delete(region);
              else next.add(region);
              return { ...current, enabled: orderedRegions(next) };
            });
          },
        };
      }),
    [enabled],
  );
}

/** Registers the commands that are always available (see {@link CommandPalette}). */
export function useAppCommands(): void {
  useCommandSource("window", useWindowCommands());
  useCommandSource("regions", useRegionCommands());
}
