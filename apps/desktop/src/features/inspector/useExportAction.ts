import { useQueryClient, type QueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useShallow } from "zustand/shallow";

import { useCapabilities } from "@/features/gpu/useCapabilities";
import {
  analyzeItem,
  cancelItem,
  exportItem,
  exportItems,
  useJobsStore,
  type RunEntry,
} from "@/features/jobs/jobs";
import { openProject } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import type { Capabilities } from "@/bindings/Capabilities";
import { codecProblem, templateProblem } from "@/lib/export-options";
import type { JobSettings } from "@/lib/job-settings";
import { logWarning } from "@/lib/log";
import { formatCount } from "@/lib/media";
import { settingsOf, useJobSettingsStore } from "@/store/job-settings";
import { useQueueStore, type QueueItem } from "@/store/queue";

import { useInspectorTarget, useLeadProject } from "./useInspector";

/** What the footer's primary button does. */
export type ExportActionKind = "analyze" | "export" | "export-many" | "cancel";

/** The footer's primary action for the current selection. */
export interface ExportAction {
  readonly kind: ExportActionKind;
  readonly label: string;
  /** Why the action cannot run (a codec or file name problem), if it cannot. */
  readonly blocked: string | null;
  readonly disabled: boolean;
  readonly run: () => void;
}

/** Why `settings` cannot be exported on this machine, if they cannot. */
function exportProblem(caps: Capabilities | null, settings: JobSettings): string | null {
  return (
    codecProblem(caps, settings.export.codec) ?? templateProblem(settings.export.output.template)
  );
}

/** Opens (or reads from the cache) the project of each file and hands them to the run queue. */
async function exportSelection(items: readonly QueueItem[], queryClient: QueryClient) {
  const settings = useJobSettingsStore.getState();
  let entries: RunEntry[];
  try {
    entries = await Promise.all(
      items.map(async (item) => {
        const project = await queryClient.fetchQuery({
          queryKey: ["project", item.path],
          queryFn: () => openProject(item.path),
        });
        return { item, projectId: project.id, settings: settingsOf(settings, item.id) };
      }),
    );
  } catch (reason) {
    logWarning("opening the projects to export failed", reason);
    toast.error("The files could not be exported", { description: toAppError(reason).message });
    return;
  }
  await exportItems(entries, queryClient);
}

/**
 * The inspector footer's primary action (design 8.1): Cancel while the
 * shown file has an operation; "Export N files" when several idle videos
 * are selected (the run queue analyses the ones that need it); otherwise
 * Analyze before the shown file's first analysis and Export (again)
 * after it. Ctrl+E runs the same action.
 */
export function useExportAction(): ExportAction {
  const { lead, settings } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const caps = useCapabilities().data ?? null;
  const queryClient = useQueryClient();
  // Only what the action depends on, so job progress does not render it again.
  const running = useJobsStore((state) =>
    lead ? state.running[lead.id]?.progress.operation : undefined,
  );
  const runningIds = useJobsStore(useShallow((state) => Object.keys(state.running)));
  const selectedIds = useQueueStore(
    useShallow((queue) =>
      queue.items
        .filter((item) => queue.selected.has(item.id) && item.media)
        .map((item) => item.id),
    ),
  );
  const fileSettings = useJobSettingsStore((state) => state);

  if (running) {
    return {
      kind: "cancel",
      label: running === "analyze" ? "Cancel analysis" : "Cancel export",
      blocked: null,
      disabled: false,
      run: () => void cancelItem(lead?.id ?? ""),
    };
  }

  const idle = selectedIds.filter((id) => !runningIds.includes(id));
  if (idle.length > 1) {
    const blocked =
      idle.map((id) => exportProblem(caps, settingsOf(fileSettings, id))).find(Boolean) ?? null;
    return {
      kind: "export-many",
      label: `Export ${formatCount(idle.length)} files`,
      blocked,
      disabled: blocked !== null,
      run: () => {
        const { items } = useQueueStore.getState();
        void exportSelection(
          items.filter((item) => idle.includes(item.id)),
          queryClient,
        );
      },
    };
  }

  const kind = project?.analyzed ? "export" : "analyze";
  const blocked = kind === "export" ? exportProblem(caps, settings) : null;
  const exported = lead !== null && ["done", "warnings", "unverified"].includes(lead.status.kind);
  return {
    kind,
    label: kind === "analyze" ? "Analyze" : exported ? "Export again" : "Export",
    blocked,
    disabled: !lead || !project || blocked !== null,
    run: () => {
      if (!lead || !project) return;
      if (kind === "analyze") void analyzeItem(lead, project.id, settings, queryClient);
      else {
        const target = { id: project.id, unverified: project.verify.unverified_ranges.length > 0 };
        void exportItem(lead, target, settings, queryClient);
      }
    },
  };
}
