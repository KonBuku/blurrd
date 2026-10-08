import { useState, type ComponentProps, type KeyboardEvent } from "react";

import { cn } from "@/lib/utils";

import { inputVariants } from "./input";

export type NumberFieldProps = Omit<
  ComponentProps<"input">,
  "value" | "defaultValue" | "onChange" | "type" | "size" | "min" | "max" | "step"
> & {
  readonly value: number;
  /** Called with the new value, already clamped and rounded to `step`. */
  readonly onValueChange: (value: number) => void;
  readonly min?: number;
  readonly max?: number;
  /** Arrow-key increment; Shift multiplies it by ten. Also the precision. */
  readonly step?: number;
  /** Unit shown after the number, for example "px" or "%". */
  readonly unit?: string;
  readonly size?: "xs" | "sm" | "md" | "lg";
};

/** Number of decimals of `step` (0.25 -> 2). */
function decimals(step: number): number {
  const text = String(step);
  const dot = text.indexOf(".");
  return dot === -1 ? 0 : text.length - dot - 1;
}

/** Clamps `value` into [min, max] and rounds it to multiples of `step`. */
export function normalizeNumber(value: number, min: number, max: number, step: number): number {
  const rounded = Math.round(value / step) * step;
  const clamped = Math.min(Math.max(rounded, min), max);
  return Number(clamped.toFixed(decimals(step)));
}

/**
 * A compact numeric field (padding, feather, NVENC sessions): right-aligned
 * tabular digits, Arrow Up/Down step by `step` (×10 with Shift), Home/End
 * jump to the limits. Typed text is committed on Enter or blur; text that
 * is not a number reverts to the last value, Escape cancels the edit.
 */
export function NumberField({
  value,
  onValueChange,
  min = Number.NEGATIVE_INFINITY,
  max = Number.POSITIVE_INFINITY,
  step = 1,
  unit,
  size = "sm",
  className,
  onBlur,
  onKeyDown,
  disabled,
  ...props
}: NumberFieldProps) {
  // The text being typed, or null while the field shows `value`.
  const [draft, setDraft] = useState<string | null>(null);

  const commit = (next: number) => {
    setDraft(null);
    const normalized = normalizeNumber(next, min, max, step);
    if (normalized !== value) onValueChange(normalized);
  };

  const commitDraft = () => {
    if (draft === null) return;
    const parsed = Number(draft.trim().replace(",", "."));
    if (draft.trim() === "" || !Number.isFinite(parsed)) setDraft(null);
    else commit(parsed);
  };

  const handleKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    onKeyDown?.(event);
    if (event.defaultPrevented) return;
    const factor = event.shiftKey ? 10 : 1;
    const current = draft === null ? value : Number(draft) || value;
    switch (event.key) {
      case "ArrowUp":
        event.preventDefault();
        commit(current + step * factor);
        break;
      case "ArrowDown":
        event.preventDefault();
        commit(current - step * factor);
        break;
      case "Home":
        if (Number.isFinite(min)) {
          event.preventDefault();
          commit(min);
        }
        break;
      case "End":
        if (Number.isFinite(max)) {
          event.preventDefault();
          commit(max);
        }
        break;
      case "Enter":
        commitDraft();
        break;
      case "Escape":
        if (draft !== null) {
          event.preventDefault();
          setDraft(null);
        }
        break;
      default:
        break;
    }
  };

  return (
    <div
      data-slot="number-field"
      className={cn(
        inputVariants({ size }),
        "flex items-center gap-1 focus-within:border-input-border-focused focus-within:bg-input-focused has-focus-visible:outline-2 has-focus-visible:outline-offset-1 has-focus-visible:outline-ring",
        disabled && "pointer-events-none opacity-50",
        className,
      )}
    >
      <input
        type="text"
        inputMode="decimal"
        role="spinbutton"
        autoComplete="off"
        spellCheck={false}
        aria-valuenow={value}
        aria-valuemin={Number.isFinite(min) ? min : undefined}
        aria-valuemax={Number.isFinite(max) ? max : undefined}
        aria-valuetext={unit ? `${value} ${unit}` : undefined}
        disabled={disabled}
        value={draft ?? value.toFixed(decimals(step))}
        onChange={(event) => setDraft(event.target.value)}
        onBlur={(event) => {
          commitDraft();
          onBlur?.(event);
        }}
        onKeyDown={handleKeyDown}
        className="w-full min-w-0 bg-transparent text-right font-mono tabular-nums outline-none"
        {...props}
      />
      {unit && <span className="shrink-0 text-ui-sm text-foreground-subtle">{unit}</span>}
    </div>
  );
}
