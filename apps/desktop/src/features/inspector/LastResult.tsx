import { FileSearch } from "lucide-react";
import { useState } from "react";

import type { ProjectSummary } from "@/bindings/ProjectSummary";
import { Button } from "@/components/ui/button";
import { RegionChip } from "@/components/ui/region-chip";
import { useControllerStore } from "@/features/preview/playback";
import { useVerifyReport } from "@/features/timeline/useTimelineData";
import { formatTimecode } from "@/lib/frames";
import { formatCount } from "@/lib/media";
import { STATUS_LABELS, countedLeaks } from "@/lib/verify-report";
import { usePreviewStore } from "@/store/preview";
import type { QueueItem } from "@/store/queue";

import { InspectorRow, InspectorSection } from "./parts";
import { ReportDialog } from "./ReportDialog";
import { ResultBadge } from "./ResultBadge";

/** Leak rows listed in the card; the report dialog lists all. */
const MAX_ROWS = 5;

export interface LastResultProps {
  readonly item: QueueItem;
  readonly project: ProjectSummary;
}

/**
 * The last verification of the shown file (design 8.1 "Last result card"):
 * the result, its counts, the ranges edits changed since, the first leaks
 * (a row jumps the preview to the leak and selects it) and the full
 * report.
 */
export function LastResult({ item, project }: LastResultProps) {
  const report = useVerifyReport(project.id).data ?? null;
  const controller = useControllerStore((state) => state.controller);
  const [open, setOpen] = useState(false);
  const [num, den] = project.media.fps;
  const time = (frame: number) => formatTimecode((frame * den) / num);
  const unverified = project.verify.unverified_ranges;

  if (!report) {
    return (
      <InspectorSection title="Last result">
        <InspectorRow
          label="Not verified yet"
          description="The verification runs with the analysis of this recording."
        />
      </InspectorSection>
    );
  }

  const leaks = countedLeaks(report.leaks);
  const { summary } = report;
  return (
    <InspectorSection title="Last result" aside={<ResultBadge summary={report.summary} />}>
      <InspectorRow
        label={`${formatCount(report.frames_checked)} frames checked`}
        description={[
          `${summary.open} open`,
          `${summary.fixed} fixed`,
          `${summary.accepted} accepted`,
          summary.out_of_scope > 0 ? `${summary.out_of_scope} outside the regions` : null,
        ]
          .filter((part) => part !== null)
          .join(" · ")}
      />
      {unverified.length > 0 && (
        <InspectorRow
          label="Edited since"
          description={`${unverified.length === 1 ? "One range" : `${unverified.length} ranges`} changed by edits ${unverified.length === 1 ? "is" : "are"} verified again before the export.`}
        />
      )}
      {leaks.length > 0 && (
        <ul aria-label="Leaks" className="flex flex-col border-t border-border">
          {leaks.slice(0, MAX_ROWS).map(({ index, leak }) => (
            <li key={index}>
              <button
                type="button"
                disabled={!controller}
                onClick={() => {
                  controller?.seek(leak.frame_range[0]);
                  usePreviewStore.getState().select(`leak:${index}`);
                }}
                className="flex w-full cursor-default items-center gap-2 px-3 py-1.5 text-left hover:bg-hover disabled:opacity-50"
              >
                <span className="font-mono text-ui-sm tabular-nums">
                  {time(leak.frame_range[0])}
                </span>
                {leak.region && <RegionChip region={leak.region} />}
                <span className="ml-auto text-ui-sm text-foreground-subtle">
                  {STATUS_LABELS[leak.status]}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      <div className="border-t border-border px-3 py-2">
        <Button variant="outline" size="sm" onClick={() => setOpen(true)}>
          <FileSearch strokeWidth={1.5} />
          Open full report
        </Button>
      </div>
      <ReportDialog
        open={open}
        onOpenChange={setOpen}
        item={item}
        project={project}
        report={report}
      />
    </InspectorSection>
  );
}
