import { keepPreviousData, useQuery } from "@tanstack/react-query";

import type { ExportSettings } from "@/bindings/ExportSettings";
import type { ProjectSummary } from "@/bindings/ProjectSummary";
import { useDebounced } from "@/hooks/useDebounced";
import { getOutputPreview } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { templateProblem } from "@/lib/export-options";
import { baseName, dirName, formatBytes, formatDuration } from "@/lib/media";

import { InspectorRow, InspectorSection } from "./parts";

/** Settle time of the export settings before the estimate is read again. */
const DEBOUNCE_MS = 150;

export interface OutputEstimateProps {
  /** The shown file's project; `null` without one. */
  readonly project: ProjectSummary | null;
  readonly settings: ExportSettings;
}

/**
 * The output file name and the size and time estimate of the shown file
 * with the current export settings (`get_output_preview`), read again a
 * moment after the settings stop changing.
 */
export function OutputEstimate({ project, settings }: OutputEstimateProps) {
  const debounced = useDebounced(settings, DEBOUNCE_MS);
  const valid = templateProblem(debounced.output.template) === null;
  const preview = useQuery({
    queryKey: ["output-preview", project?.id, debounced],
    queryFn: () => getOutputPreview(project?.id ?? "", debounced),
    enabled: project !== null && valid,
    placeholderData: keepPreviousData,
    staleTime: 0,
  });

  return (
    <InspectorSection title="Output">
      {project === null ? (
        <InspectorRow
          label="No recording selected"
          description="Select a recording to see its output name and size."
        />
      ) : preview.error ? (
        <InspectorRow
          label="The output could not be estimated"
          description={toAppError(preview.error).message}
        />
      ) : (
        <div data-testid="output-preview" aria-live="polite" aria-busy={preview.isFetching}>
          <InspectorRow
            label={
              <span className="block font-mono break-all" title={preview.data?.path}>
                {preview.data ? baseName(preview.data.path) : "…"}
              </span>
            }
            description={
              preview.data
                ? `About ${formatBytes(preview.data.est_size)} · ${durationText(preview.data.est_seconds)}`
                : "Estimating…"
            }
          />
          <InspectorRow
            label="Folder"
            description={
              <span className="flex min-w-0 flex-col gap-0.5">
                <span
                  data-testid="output-folder"
                  className="block font-mono break-all text-foreground"
                  title={preview.data ? dirName(preview.data.path) : undefined}
                >
                  {preview.data ? dirName(preview.data.path) : "…"}
                </span>
                <span>
                  {typeof settings.output.folder === "string"
                    ? "Next to the recording."
                    : "The chosen export folder."}
                </span>
              </span>
            }
          />
        </div>
      )}
    </InspectorSection>
  );
}

/** How long an export takes, in words. */
function durationText(seconds: number): string {
  return seconds < 1
    ? "under a second to export"
    : `about ${formatDuration(Math.ceil(seconds))} to export`;
}
