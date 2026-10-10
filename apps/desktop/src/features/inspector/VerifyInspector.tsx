import { useId } from "react";

import type { Audit } from "@/bindings/Audit";
import type { Coverage } from "@/bindings/Coverage";
import type { VerifyMode } from "@/bindings/VerifyMode";
import type { VerifySettings } from "@/bindings/VerifySettings";
import { NumberField } from "@/components/ui/number-field";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { DEFAULT_VERIFY } from "@/lib/job-settings";

import { LastResult } from "./LastResult";
import { Disclosure, InspectorRow, InspectorSection, SliderRow, WordListInput } from "./parts";
import { useInspectorTarget, useLeadProject } from "./useInspector";

/** Interval range of the full-frame sample for names outside every region, in ms. */
const SAMPLE_MS = { min: 100, max: 5000, step: 100 } as const;

const MODE_HINTS: Readonly<Record<VerifyMode, string>> = {
  DetectAndFix: "Covers every name OCR can still read with its whole name slot and checks again.",
  ReportOnly: "Only lists what OCR can still read; nothing is added.",
};

const COVERAGE_HINTS: Readonly<Record<Coverage, string>> = {
  Structure: "Every frame with HUD activity, and one frame in 15 elsewhere.",
  EveryFrame: "Every frame; slower, recommended before publishing.",
};

const AUDIT_HINTS: Readonly<Record<Audit, string>> = {
  Off: "The exported file is not read again.",
  Sampled: "Reads sampled frames of the exported file again.",
  EveryFrame: "Reads every frame of the exported file again.",
};

/**
 * Inspector › Verify (design 8.1): OCR verification on or off, fix or
 * report only, coverage, confidence threshold, words that never count as
 * names, the output audit, and the last result of the shown file with the
 * full report. The settings apply to the next analysis (plan 7.2).
 */
export function VerifyInspector() {
  const { lead, settings, update } = useInspectorTarget();
  const { project } = useLeadProject(lead);
  const ocr = useCapabilities().data?.ocr ?? "NotProbed";
  const verify = settings.verify;
  const change = (patch: (current: VerifySettings) => VerifySettings) => update("verify", patch);
  const set = <K extends keyof VerifySettings>(key: K, value: VerifySettings[K]) =>
    change((current) => ({ ...current, [key]: value }));
  const enabledId = useId();
  const enabledHint = useId();
  const wordsId = useId();

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
        <InspectorRow label="Mode" description={MODE_HINTS[verify.mode]}>
          <ToggleGroup<VerifyMode>
            size="sm"
            aria-label="Verification mode"
            disabled={!verify.enabled}
            value={verify.mode}
            options={[
              { value: "DetectAndFix", label: "Detect and fix" },
              { value: "ReportOnly", label: "Report only" },
            ]}
            onValueChange={(mode) => set("mode", mode)}
          />
        </InspectorRow>
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
        <InspectorRow label="Audit the export" description={AUDIT_HINTS[verify.audit_output]}>
          <ToggleGroup<Audit>
            size="sm"
            aria-label="Audit the export"
            value={verify.audit_output}
            options={[
              { value: "Off", label: "Off" },
              { value: "Sampled", label: "Sampled" },
              { value: "EveryFrame", label: "Every frame" },
            ]}
            onValueChange={(audit) => set("audit_output", audit)}
          />
        </InspectorRow>
        <InspectorRow
          label={<label htmlFor={wordsId}>Ignore words</label>}
          description="Callouts and place names that are never treated as names. Comma separated."
        >
          <WordListInput
            id={wordsId}
            words={verify.ignore_words}
            onChange={(words) => set("ignore_words", words)}
          />
        </InspectorRow>
      </InspectorSection>
      <Disclosure label="Advanced verification">
        <InspectorSection title="Recognition">
          <SliderRow
            label="Confidence threshold"
            valueText={verify.min_conf.toFixed(2)}
            min={0.3}
            max={0.9}
            step={0.05}
            value={verify.min_conf}
            resetValue={DEFAULT_VERIFY.min_conf}
            onValueChange={(conf) => set("min_conf", Number(conf.toFixed(2)))}
            caption={["More reported", "Fewer reported"]}
          />
          <InspectorRow
            label="Names outside the regions"
            description="How often the whole frame is read for names outside every region (listed for information only)."
            control={
              <NumberField
                aria-label="Full-frame sample interval"
                className="w-24"
                unit="ms"
                min={SAMPLE_MS.min}
                max={SAMPLE_MS.max}
                step={SAMPLE_MS.step}
                value={verify.full_frame_sample_ms}
                onValueChange={(ms) => set("full_frame_sample_ms", ms)}
              />
            }
          />
        </InspectorSection>
      </Disclosure>
    </div>
  );
}
