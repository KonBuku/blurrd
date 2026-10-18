import { useQueryClient } from "@tanstack/react-query";
import { RefreshCw } from "lucide-react";

import { Button } from "@/components/ui/button";
import { analyzeItem, useJobsStore } from "@/features/jobs/jobs";

import { DetectionSection } from "./DetectionSection";
import { ManualBoxes } from "./ManualBoxes";
import { Disclosure } from "./parts";
import { RegionsSection } from "./RegionsSection";
import { StyleSection } from "./StyleSection";
import { useInspectorTarget, useLeadProject } from "./useInspector";

/**
 * Inspector › Blur (design 8.1): regions, manual boxes, style and the
 * advanced detection options with Re-analyze. Every change goes to the
 * inspector's targets; the preview follows the shown file at once.
 */
export function BlurInspector() {
  const { lead, settings, update } = useInspectorTarget();
  const { project, tracks } = useLeadProject(lead);
  const queryClient = useQueryClient();
  const running = useJobsStore((state) => (lead ? lead.id in state.running : false));

  return (
    <div className="flex flex-col gap-4">
      <RegionsSection
        regions={settings.regions}
        tracks={project?.analyzed ? (tracks ?? null) : null}
        onChange={(change) => update("regions", change)}
      />
      {project?.analyzed && <ManualBoxes project={project} />}
      <StyleSection blur={settings.blur} onChange={(change) => update("blur", change)} />
      <Disclosure label="Advanced detection">
        <DetectionSection
          mode={settings.regions.mode}
          onModeChange={(mode) => update("regions", (regions) => ({ ...regions, mode }))}
        />
        {lead && project && (
          <div className="flex flex-col gap-1 px-1">
            <Button
              variant="secondary"
              className="self-start"
              disabled={running}
              aria-busy={running}
              onClick={() => void analyzeItem(lead, project.id, settings, queryClient)}
            >
              <RefreshCw strokeWidth={1.5} />
              {project.analyzed ? "Re-analyze" : "Analyze"}
            </Button>
            <p className="text-ui-sm text-foreground-subtle">
              Regions and detection options apply to the next analysis of {lead.name}.
            </p>
          </div>
        )}
      </Disclosure>
    </div>
  );
}
