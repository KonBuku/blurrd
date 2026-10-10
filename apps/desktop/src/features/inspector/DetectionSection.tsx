import { useId } from "react";

import type { Mode } from "@/bindings/Mode";
import type { Sensitivity } from "@/bindings/Sensitivity";
import type { TemporalParams } from "@/bindings/TemporalParams";
import { NumberField } from "@/components/ui/number-field";
import { ToggleGroup } from "@/components/ui/toggle-group";
import type { DetectorOptions } from "@/lib/job-settings";

import { InspectorRow, InspectorSection } from "./parts";

/** Largest temporal values offered, in frames (four seconds at 60 fps). */
const MAX_FRAMES = 240;

const MODE_HINTS: Readonly<Record<Mode, string>> = {
  Tight: "Boxes follow each name closely; unsure frames get the whole name slot.",
  Safe: "Every box covers its whole name slot.",
};

const SENSITIVITY_HINTS: Readonly<Record<Sensitivity, string>> = {
  Low: "Fewer candidates, fewer false boxes.",
  Balanced: "The tuned default.",
  High: "More candidates, more false boxes.",
};

export interface DetectionSectionProps {
  readonly mode: Mode;
  readonly detector: DetectorOptions;
  readonly onModeChange: (mode: Mode) => void;
  readonly onDetectorChange: (change: (detector: DetectorOptions) => DetectorOptions) => void;
}

/**
 * Inspector › Blur › Advanced detection: box fit, sensitivity and the
 * temporal safety margins. They take effect with the next analysis.
 */
export function DetectionSection({
  mode,
  detector,
  onModeChange,
  onDetectorChange,
}: DetectionSectionProps) {
  const modeId = useId();
  const sensitivityId = useId();
  const setTemporal = (key: keyof TemporalParams, value: number) =>
    onDetectorChange((current) => ({
      ...current,
      temporal: { ...current.temporal, [key]: value },
    }));

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
      <InspectorRow
        label="Sensitivity"
        labelId={sensitivityId}
        description={SENSITIVITY_HINTS[detector.sensitivity]}
      >
        <ToggleGroup<Sensitivity>
          size="sm"
          aria-label="Sensitivity"
          value={detector.sensitivity}
          options={[
            { value: "Low", label: "Low" },
            { value: "Balanced", label: "Balanced" },
            { value: "High", label: "High" },
          ]}
          onValueChange={(sensitivity) =>
            onDetectorChange((current) => ({ ...current, sensitivity }))
          }
        />
      </InspectorRow>
      <FramesRow
        label="Start early"
        description="Frames a box starts before the name is first seen."
        value={detector.temporal.pre_roll}
        onChange={(value) => setTemporal("pre_roll", value)}
      />
      <FramesRow
        label="Stay at least"
        description="Frames a box stays after the name was last seen."
        value={detector.temporal.post_roll_min}
        onChange={(value) => setTemporal("post_roll_min", value)}
      />
      <FramesRow
        label="Bridge gaps up to"
        description="Keeps boxes through flashes and slide-in animations."
        value={detector.temporal.max_gap}
        onChange={(value) => setTemporal("max_gap", value)}
      />
    </InspectorSection>
  );
}

interface FramesRowProps {
  readonly label: string;
  readonly description: string;
  readonly value: number;
  readonly onChange: (value: number) => void;
}

/** A frame count with its explanation. */
function FramesRow({ label, description, value, onChange }: FramesRowProps) {
  const descriptionId = useId();
  return (
    <InspectorRow
      label={label}
      description={description}
      descriptionId={descriptionId}
      control={
        <NumberField
          aria-label={label}
          aria-describedby={descriptionId}
          className="w-24"
          unit="frames"
          min={0}
          max={MAX_FRAMES}
          value={value}
          onValueChange={onChange}
        />
      }
    />
  );
}
