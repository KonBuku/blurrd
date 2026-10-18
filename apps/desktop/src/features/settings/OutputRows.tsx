import { useId, useState } from "react";

import { Input } from "@/components/ui/input";
import { NumberField } from "@/components/ui/number-field";
import { formatMbps, templateProblem } from "@/lib/export-options";

import { SettingsRow } from "./SettingsRow";

/** Largest VBR rate offered, in Mbit/s. */
const MAX_MBPS = 400;

export interface TemplateRowProps {
  readonly template: string;
  readonly extension: string;
  readonly onChange: (template: string) => void;
}

/**
 * The file name template: `{name}` stands for the recording's file name.
 * Checked as it is typed and saved when valid (on blur or Enter); Escape
 * restores the saved one.
 */
export function TemplateRow({ template, extension, onChange }: TemplateRowProps) {
  const id = useId();
  const hintId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? template;
  const problem = templateProblem(text);
  const commit = () => {
    if (draft === null || problem !== null) return;
    if (draft.trim() !== template) onChange(draft.trim());
    setDraft(null);
  };
  return (
    <SettingsRow
      label={<label htmlFor={id}>File name</label>}
      controlWidth="wide"
      description={
        <span id={hintId}>
          {problem ? (
            <span className="text-destructive">{problem}</span>
          ) : (
            <>
              <code className="font-mono">{"{name}"}</code> is the recording&apos;s file name, for
              example{" "}
              <span className="font-mono">{`${text.trim().replaceAll("{name}", "match")}.${extension}`}</span>
              .
            </>
          )}
        </span>
      }
      control={
        <Input
          id={id}
          size="lg"
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem !== null}
          aria-describedby={hintId}
          value={text}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape" && draft !== null) {
              event.preventDefault();
              event.stopPropagation();
              setDraft(null);
            }
          }}
          className="w-full font-mono"
        />
      }
    />
  );
}

export interface VbrRowsProps {
  readonly target: number;
  readonly peak: number;
  readonly onChange: (target: number, peak: number) => void;
}

/** Target and peak bit rate, in Mbit/s; the peak never drops below the target. */
export function VbrRows({ target, peak, onChange }: VbrRowsProps) {
  const targetMbps = Number(formatMbps(target));
  return (
    <>
      <SettingsRow
        label="Target bit rate"
        control={
          <NumberField
            size="lg"
            aria-label="Target bit rate"
            unit="Mbit/s"
            value={targetMbps}
            min={1}
            max={MAX_MBPS}
            onValueChange={(mbps) => {
              const kbps = mbps * 1000;
              onChange(kbps, Math.max(peak, kbps));
            }}
            className="w-32"
          />
        }
      />
      <SettingsRow
        label="Peak bit rate"
        control={
          <NumberField
            size="lg"
            aria-label="Peak bit rate"
            unit="Mbit/s"
            value={Number(formatMbps(peak))}
            min={targetMbps}
            max={MAX_MBPS}
            onValueChange={(mbps) => onChange(target, Math.max(mbps * 1000, target))}
            className="w-32"
          />
        }
      />
    </>
  );
}
