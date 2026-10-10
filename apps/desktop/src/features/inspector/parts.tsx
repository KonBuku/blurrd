import { ChevronRight } from "lucide-react";
import { useId, useState, type ReactNode } from "react";

import { Slider } from "@/components/ui/slider";
import { cn } from "@/lib/utils";

export interface InspectorSectionProps {
  /** Section title, shown in small capitals above the card. */
  readonly title: string;
  /** Controls next to the title (for example a count). */
  readonly aside?: ReactNode;
  readonly children: ReactNode;
}

/**
 * A titled group of inspector rows (design 8.1: REGIONS, STYLE, …): an
 * uppercase caption and a bordered card whose rows are separated by
 * hairlines. The section is a landmark-free `section` named by its title.
 */
export function InspectorSection({ title, aside, children }: InspectorSectionProps) {
  const id = useId();
  return (
    <section aria-labelledby={id} className="flex flex-col gap-1.5">
      <div className="flex items-center justify-between gap-2 px-1">
        <h2
          id={id}
          className="text-ui-xs font-medium tracking-wide text-foreground-subtle uppercase"
        >
          {title}
        </h2>
        {aside}
      </div>
      <div className="flex flex-col overflow-hidden rounded-lg border border-card-border bg-card">
        {children}
      </div>
    </section>
  );
}

export interface InspectorRowProps {
  /** Row label; named by `labelId` for the control's `aria-labelledby`. */
  readonly label: ReactNode;
  readonly labelId?: string;
  /** Short explanation below the label. */
  readonly description?: ReactNode;
  /** Id of the description, for the control's `aria-describedby`. */
  readonly descriptionId?: string;
  /** Leading visual (a region icon). */
  readonly icon?: ReactNode;
  /** Control at the end of the row. */
  readonly control?: ReactNode;
  /** Full-width content below the label and control (a slider, options). */
  readonly children?: ReactNode;
  /** Indents the row under the one above (sub-options). */
  readonly nested?: boolean;
  readonly className?: string;
}

/**
 * One inspector setting: icon, label and description on the left, the
 * control at the end, optional full-width content below. Rows inside an
 * {@link InspectorSection} get a hairline between them.
 */
export function InspectorRow({
  label,
  labelId,
  description,
  descriptionId,
  icon,
  control,
  children,
  nested = false,
  className,
}: InspectorRowProps) {
  return (
    <div
      data-slot="inspector-row"
      className={cn(
        "flex flex-col gap-2 border-t border-border px-3 py-2.5 first:border-t-0",
        nested && "border-t-0 pt-0 pl-9",
        className,
      )}
    >
      <div className="flex min-w-0 items-center gap-2">
        {icon}
        <div className="min-w-0 flex-1">
          <div id={labelId} className={cn("text-ui-base", !nested && "font-medium")}>
            {label}
          </div>
          {description && (
            <div id={descriptionId} className="text-ui-sm text-foreground-subtle">
              {description}
            </div>
          )}
        </div>
        {control !== undefined && <div className="flex shrink-0 items-center gap-2">{control}</div>}
      </div>
      {children}
    </div>
  );
}

export interface SliderRowProps {
  readonly label: string;
  readonly value: number;
  readonly min: number;
  readonly max: number;
  readonly step?: number;
  /** Unit after the value, for example "px". */
  readonly unit: string;
  /** Value a double-click on the thumb restores. */
  readonly resetValue: number;
  readonly onValueChange: (value: number) => void;
  readonly disabled?: boolean;
}

/** A labelled slider row with its value at the end (design 8.1 STYLE). */
export function SliderRow({
  label,
  value,
  min,
  max,
  step = 1,
  unit,
  resetValue,
  onValueChange,
  disabled = false,
}: SliderRowProps) {
  const labelId = useId();
  return (
    <InspectorRow
      label={label}
      labelId={labelId}
      control={
        // The slider itself reports the value to assistive technology.
        <span aria-hidden className="font-mono text-ui-sm text-foreground-subtle tabular-nums">
          {value} {unit}
        </span>
      }
    >
      <Slider
        aria-labelledby={labelId}
        min={min}
        max={max}
        step={step}
        value={[value]}
        resetValue={[resetValue]}
        disabled={disabled}
        onValueChange={([next]) => {
          if (next !== undefined) onValueChange(next);
        }}
      />
    </InspectorRow>
  );
}

export interface DisclosureProps {
  readonly label: string;
  readonly children: ReactNode;
  readonly defaultOpen?: boolean;
}

/** A collapsible block with a chevron button (design 8.1 "▸ Detection (adv.)"). */
export function Disclosure({ label, children, defaultOpen = false }: DisclosureProps) {
  const [open, setOpen] = useState(defaultOpen);
  const id = useId();
  return (
    <div className="flex flex-col gap-1.5">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className="flex h-7 cursor-default items-center gap-1 rounded-md px-1 text-ui-xs font-medium tracking-wide text-foreground-subtle uppercase hover:text-foreground"
      >
        <ChevronRight
          aria-hidden
          strokeWidth={1.5}
          className={cn("size-3.5 transition-transform", open && "rotate-90")}
        />
        {label}
      </button>
      <div id={id} hidden={!open} className="flex flex-col gap-4">
        {children}
      </div>
    </div>
  );
}
