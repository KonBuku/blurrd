import { useId } from "react";

import type { Mode } from "@/bindings/Mode";
import { ToggleGroup } from "@/components/ui/toggle-group";

import { InspectorRow, InspectorSection } from "./parts";

const MODE_HINTS: Readonly<Record<Mode, string>> = {
  Tight: "Boxes follow each name closely; unsure frames get the whole name slot.",
  Safe: "Every box covers its whole name slot.",
};

export interface DetectionSectionProps {
  readonly mode: Mode;
  readonly onModeChange: (mode: Mode) => void;
}

/**
 * Inspector › Blur › Advanced detection: the box fit. It takes effect
 * with the next analysis: Re-analyze, or an "Export N files" run, which
 * analyses a file with Tight boxes again when Safe is chosen.
 */
export function DetectionSection({ mode, onModeChange }: DetectionSectionProps) {
  const modeId = useId();
  return (
    <InspectorSection title="Detection">
      <InspectorRow label="Box fit" labelId={modeId} description={MODE_HINTS[mode]}>
        <ToggleGroup<Mode>
          size="sm"
          aria-label="Box fit"
          value={mode}
          options={[
            { value: "Tight", label: "Tight" },
            { value: "Safe", label: "Safe" },
          ]}
          onValueChange={onModeChange}
        />
      </InspectorRow>
    </InspectorSection>
  );
}
