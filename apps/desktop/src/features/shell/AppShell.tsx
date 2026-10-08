import { useEffect, useState } from "react";

import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet";
import { INSPECTOR_ID, InspectorTabs } from "@/features/inspector/Inspector";
import { INSPECTOR_TOGGLE_ID, PreviewFrame } from "@/features/preview/PreviewFrame";
import { QUEUE_SIDEBAR_ID, QueueSidebar } from "@/features/queue/QueueSidebar";
import { SettingsView } from "@/features/settings/SettingsView";
import { useWindowWidth } from "@/hooks/useWindowWidth";
import { INSPECTOR, INSPECTOR_SHEET_BREAKPOINT, SIDEBAR, clamp, resolveLayout } from "@/lib/layout";
import { matchShortcut } from "@/lib/shortcuts";
import { useUiStore } from "@/store/ui";

import { Frame } from "./Frame";
import { ResizeHandle } from "./ResizeHandle";
import { TitleBar } from "./TitleBar";

/**
 * The main window (design 8.0): title bar on top; below it the queue
 * sidebar on the shell colour, the preview frame and the inspector frame,
 * separated by 4 px gaps that double as resize handles. Below 1180 px the
 * inspector becomes an overlay sheet toggled from the preview header.
 * The settings view replaces the preview and inspector frames.
 */
export function AppShell() {
  const windowWidth = useWindowWidth();
  const ui = useUiStore();
  const layout = resolveLayout(windowWidth, ui);
  const [resizing, setResizing] = useState(false);

  const inspectorShown = layout.inspectorAsSheet ? ui.inspectorSheetOpen : ui.inspectorOpen;
  const toggleInspector = () => ui.toggleInspector(layout.inspectorAsSheet);

  // The sheet belongs to narrow windows; growing the window closes it.
  const { inspectorAsSheet } = layout;
  const { setInspectorSheetOpen } = ui;
  useEffect(() => {
    if (!inspectorAsSheet) setInspectorSheetOpen(false);
  }, [inspectorAsSheet, setInspectorSheetOpen]);

  useGlobalShortcuts();

  const sidebarOpenWidth = clamp(ui.sidebarWidth, SIDEBAR.min, layout.sidebarMax);

  return (
    <div className="flex h-full flex-col overflow-hidden bg-background-win-alt text-foreground">
      <TitleBar sidebarOpen={ui.sidebarOpen} onToggleSidebar={ui.toggleSidebar} />
      <div className="flex min-h-0 flex-1">
        <QueueSidebar open={ui.sidebarOpen} width={sidebarOpenWidth} resizing={resizing} />
        <main className="flex min-w-0 flex-1 pr-1 pb-1">
          {ui.sidebarOpen ? (
            <ResizeHandle
              label="Resize queue"
              controls={QUEUE_SIDEBAR_ID}
              value={sidebarOpenWidth}
              min={SIDEBAR.min}
              max={layout.sidebarMax}
              panel="left"
              onResize={ui.setSidebarWidth}
              onReset={() => ui.setSidebarWidth(SIDEBAR.default)}
              onDraggingChange={setResizing}
            />
          ) : (
            <div className="w-1 shrink-0" />
          )}
          {ui.view === "settings" && <SettingsView />}
          {ui.view === "workspace" && (
            <PreviewFrame inspectorShown={inspectorShown} onToggleInspector={toggleInspector} />
          )}
          {ui.view === "workspace" && !layout.inspectorAsSheet && ui.inspectorOpen && (
            <>
              <ResizeHandle
                label="Resize inspector"
                controls={INSPECTOR_ID}
                value={layout.inspectorWidth}
                min={INSPECTOR.min}
                max={layout.inspectorMax}
                panel="right"
                onResize={ui.setInspectorWidth}
                onReset={() => ui.setInspectorWidth(INSPECTOR.default)}
              />
              <Frame
                id={INSPECTOR_ID}
                aria-label="Inspector"
                className="shrink-0"
                style={{ width: layout.inspectorWidth }}
              >
                <InspectorTabs />
              </Frame>
            </>
          )}
        </main>
      </div>
      {layout.inspectorAsSheet && (
        <Sheet open={ui.inspectorSheetOpen} onOpenChange={ui.setInspectorSheetOpen}>
          <SheetContent
            id={INSPECTOR_ID}
            style={{ width: layout.inspectorWidth }}
            onCloseAutoFocus={(event) => {
              // The sheet has no trigger element; hand focus back to the
              // header button that opened it.
              event.preventDefault();
              document.getElementById(INSPECTOR_TOGGLE_ID)?.focus();
            }}
          >
            <InspectorTabs
              header={
                <div className="flex h-10 shrink-0 items-center border-b border-border pr-10 pl-3">
                  <SheetTitle>Inspector</SheetTitle>
                  <SheetDescription className="sr-only">
                    Blur, export and verification settings
                  </SheetDescription>
                </div>
              }
            />
          </SheetContent>
        </Sheet>
      )}
    </div>
  );
}

/**
 * Handles the window-wide shortcuts. Events another handler already
 * consumed (menus, dialogs, text fields that use the keys) are ignored.
 */
function useGlobalShortcuts() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented) return;
      const match = matchShortcut(event);
      const ui = useUiStore.getState();
      switch (match?.id) {
        case "toggleQueue":
          ui.toggleSidebar();
          break;
        case "toggleInspector":
          if (ui.view !== "workspace") return;
          ui.toggleInspector(window.innerWidth < INSPECTOR_SHEET_BREAKPOINT);
          break;
        case "openSettings":
          ui.openSettings();
          break;
        case "closeOverlay":
          // Open menus and dialogs handle Escape first (defaultPrevented).
          if (ui.view !== "settings") return;
          ui.closeSettings();
          break;
        case undefined:
          return;
      }
      event.preventDefault();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
