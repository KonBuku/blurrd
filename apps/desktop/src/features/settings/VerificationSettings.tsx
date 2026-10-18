import { useId } from "react";

import type { OcrCapability } from "@/bindings/OcrCapability";
import { Switch } from "@/components/ui/switch";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { useJobSettingsStore } from "@/store/job-settings";

import { ReadOnlyNotice, SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** What OCR can do on this PC, in a sentence. */
function ocrText(ocr: OcrCapability | undefined): string {
  if (ocr === undefined || ocr === "NotProbed") {
    return "OCR is installed; it starts with the first analysis.";
  }
  if ("Unavailable" in ocr) {
    return `OCR is not available (${ocr.Unavailable.reason}), so exports are named _UNVERIFIED.`;
  }
  return `OCR runs on ${ocr.Available.provider === "Cpu" ? "the CPU" : `${ocr.Available.provider} (GPU)`}.`;
}

/**
 * Settings › Verification (plan 6.3 `defaults.verify`, 7.2): whether new
 * files are verified with OCR, what is read, and what OCR can do on this
 * PC. The engine reads only the switch (it always fixes what it can still
 * read); mode, coverage, confidence, ignored words and the audit are not
 * read yet, so they are not offered here.
 */
export function VerificationSettings() {
  const verify = useJobSettingsStore((state) => state.defaults.values.verify);
  const updateDefaults = useJobSettingsStore((state) => state.updateDefaults);
  const ocr = useCapabilities().data?.ocr;
  const enabledLabel = useId();
  const enabledHint = useId();

  return (
    <SettingsPage
      title="Verification"
      description="How new recordings are checked after blurring. The inspector still changes this per file."
    >
      <ReadOnlyNotice />
      <SettingsGroup>
        <SettingsRow
          label="Verify with OCR"
          labelId={enabledLabel}
          description={
            <span id={enabledHint}>
              Reads the blurred regions again during the analysis and covers any name that is still
              readable. Without it, exports are named _UNVERIFIED.
            </span>
          }
          control={
            <Switch
              aria-labelledby={enabledLabel}
              aria-describedby={enabledHint}
              checked={verify.enabled}
              onCheckedChange={(enabled) =>
                updateDefaults("verify", (current) => ({ ...current, enabled }))
              }
            />
          }
        />
        <SettingsRow
          label="What is read"
          description="The HUD zones whenever their content changes and at least once a second, and the whole frame once a second. Names are read on this PC and kept only in memory; reports keep positions and counts, never text."
          control={null}
        />
        <SettingsRow label="Text recognition" description={ocrText(ocr)} control={null} />
      </SettingsGroup>
    </SettingsPage>
  );
}
