import { useEffect } from "react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { chosenEncoder, exportSummary } from "@/lib/export-options";
import { ariaKeys, matchShortcut, SHORTCUTS } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useUiStore } from "@/store/ui";

import { useExportAction } from "./useExportAction";
import { useInspectorTarget, useLeadProject } from "./useInspector";

/**
 * The inspector's sticky footer (design 8.1): the primary action for the
 * selection (see `useExportAction`) and the export summary below it.
 */
export function ExportFooter() {
  const { lead, settings } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const caps = useCapabilities().data ?? null;
  const action = useExportAction();
  const sourceCodec = (project?.media ?? lead?.media)?.video_codec ?? null;
  const shortcut = action.kind === "cancel" ? null : SHORTCUTS.exportFiles;

  return (
    <div className="flex shrink-0 flex-col gap-1.5 border-t border-border p-3">
      <Button
        size="lg"
        variant={action.kind === "cancel" ? "outline" : "primary"}
        className={cn("w-full", action.kind === "cancel" && "border-destructive text-destructive")}
        disabled={action.disabled}
        aria-keyshortcuts={shortcut ? ariaKeys(shortcut) : undefined}
        onClick={action.run}
      >
        {action.label}
        {shortcut && (
          <Kbd aria-hidden className="ml-1 bg-primary-foreground/15 text-primary-foreground">
            {shortcut.keys.join("+")}
          </Kbd>
        )}
      </Button>
      <p data-testid="export-summary" className="truncate text-ui-xs text-foreground-subtle">
        {action.blocked ??
          exportSummary(
            settings.export,
            settings.verify,
            chosenEncoder(caps, settings.export, sourceCodec),
            sourceCodec,
          )}
      </p>
    </div>
  );
}

/**
 * Ctrl+E (design 8.8): runs the footer's primary action from anywhere in
 * the workspace, also while the inspector is hidden. It never cancels.
 * A component of its own, so the selection and settings it watches only
 * render this (empty) element again.
 */
export function ExportShortcut() {
  const action = useExportAction();
  const view = useUiStore((state) => state.view);
  const enabled = view === "workspace" && action.kind !== "cancel" && !action.disabled;
  const { run } = action;

  useEffect(() => {
    if (!enabled) return undefined;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.defaultPrevented || matchShortcut(event, "window")?.id !== "exportFiles") return;
      event.preventDefault();
      run();
    };
    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, [enabled, run]);

  return null;
}
