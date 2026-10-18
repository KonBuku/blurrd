import { useId } from "react";

import type { Coverage } from "@/bindings/Coverage";
import type { VerifySettings } from "@/bindings/VerifySettings";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { useCapabilities } from "@/features/gpu/useCapabilities";

import { LastResult } from "./LastResult";
import { InspectorRow, InspectorSection } from "./parts";
import { useInspectorTarget, useLeadProject } from "./useInspector";

const COVERAGE_HINTS: Readonly<Record<Coverage, string>> = {
  Structure: "Every frame with HUD activity, and one frame in 15 elsewhere.",
  EveryFrame: "Every frame; slower, recommended before publishing.",
};

/**
 * Inspector › Verify (design 8.1): OCR verification on or off, which
 * frames it reads, and the last result of the shown file with the full
 * report. The settings apply to the next analysis (plan 7.2).
 */
export function VerifyInspector() {
  const { lead, settings, update } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const ocr = useCapabilities().data?.ocr ?? "NotProbed";
  const verify = settings.verify;
  const set = <K extends keyof VerifySettings>(key: K, value: VerifySettings[K]) =>
    update("verify", (current) => ({ ...current, [key]: value }));
  const enabledId = useId();
  const enabledHint = useId();

  return (
    <div className="flex flex-col gap-4">
      {lead && project && <LastResult item={lead} project={project} />}
      <InspectorSection title="Verification">
        <InspectorRow
          label="Verify with OCR"
          labelId={enabledId}
          description={
            typeof ocr === "object" && "Unavailable" in ocr
              ? `OCR is not available (${ocr.Unavailable.reason}). Exports are named _UNVERIFIED.`
              : "Reads the blurred regions again and fixes anything still readable."
          }
          descriptionId={enabledHint}
          control={
            <Switch
              aria-labelledby={enabledId}
              aria-describedby={enabledHint}
              checked={verify.enabled}
              onCheckedChange={(enabled) => set("enabled", enabled)}
            />
          }
        />
        <InspectorRow label="Coverage" description={COVERAGE_HINTS[verify.coverage]}>
          <ToggleGroup<Coverage>
            size="sm"
            aria-label="Coverage"
            disabled={!verify.enabled}
            value={verify.coverage}
            options={[
              { value: "Structure", label: "HUD activity" },
              { value: "EveryFrame", label: "Every frame" },
            ]}
            onValueChange={(coverage) => set("coverage", coverage)}
          />
        </InspectorRow>
      </InspectorSection>
      {lead && project?.analyzed && (
        <p className="px-1 text-ui-sm text-foreground-subtle">
          Verification settings apply to the next analysis of {lead.name}; Re-analyze is under Blur
          › Advanced detection.
        </p>
      )}
    </div>
  );
}
