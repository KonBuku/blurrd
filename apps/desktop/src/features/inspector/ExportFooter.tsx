import { useQueryClient } from "@tanstack/react-query";

import { Button } from "@/components/ui/button";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { analyzeItem, cancelItem, exportItem, useJobsStore } from "@/features/jobs/jobs";
import { chosenEncoder, codecProblem, exportSummary, templateProblem } from "@/lib/export-options";
import { cn } from "@/lib/utils";

import { useInspectorTarget, useLeadProject } from "./useInspector";

/** What the primary button does. */
type Action = "analyze" | "export" | "cancel";

/**
 * The inspector's sticky footer (design 8.1): one primary action for the
 * shown file — Analyze before the first analysis, Export (or Export
 * again) afterwards, Cancel while a job runs — and the export summary
 * below it. Exporting several files at once comes with the processing
 * queue (M27).
 */
export function ExportFooter() {
  const { lead, settings } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const caps = useCapabilities().data ?? null;
  const queryClient = useQueryClient();
  const running = useJobsStore((state) => (lead ? state.running[lead.id] : undefined));
  const sourceCodec = (project?.media ?? lead?.media)?.video_codec ?? null;

  const action: Action = running ? "cancel" : project?.analyzed ? "export" : "analyze";
  const blocked =
    action === "export"
      ? (codecProblem(caps, settings.export.codec) ??
        templateProblem(settings.export.output.template))
      : null;
  const exported = lead !== null && ["done", "warnings", "unverified"].includes(lead.status.kind);
  const label =
    action === "cancel"
      ? running?.kind === "Analyze"
        ? "Cancel analysis"
        : "Cancel export"
      : action === "analyze"
        ? "Analyze"
        : exported
          ? "Export again"
          : "Export";

  const run = () => {
    if (!lead || !project) return;
    if (action === "cancel") void cancelItem(lead.id);
    else if (action === "analyze") void analyzeItem(lead, project.id, settings, queryClient);
    else void exportItem(lead, project.id, settings, queryClient);
  };

  return (
    <div className="flex shrink-0 flex-col gap-1.5 border-t border-border p-3">
      <Button
        size="lg"
        variant={action === "cancel" ? "outline" : "primary"}
        className={cn("w-full", action === "cancel" && "border-destructive text-destructive")}
        disabled={!lead || !project || blocked !== null || (action === "cancel" && !running?.job)}
        onClick={run}
      >
        {label}
      </Button>
      <p data-testid="export-summary" className="truncate text-ui-xs text-foreground-subtle">
        {blocked ??
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
