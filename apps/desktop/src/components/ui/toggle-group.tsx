import { ToggleGroup as ToggleGroupPrimitive } from "radix-ui";
import { useLayoutEffect, useRef, useState, type ReactNode } from "react";

import { cn } from "@/lib/utils";

/** One choice of a {@link ToggleGroup}. */
export interface ToggleOption<T extends string> {
  readonly value: T;
  readonly label: string;
  /** Optional leading icon; the label stays visible next to it. */
  readonly icon?: ReactNode;
  readonly disabled?: boolean;
}

export interface ToggleGroupProps<T extends string> {
  readonly options: readonly ToggleOption<T>[];
  readonly value: T;
  readonly onValueChange: (value: T) => void;
  /** Accessible name of the group. */
  readonly "aria-label": string;
  /** `md`: 32 px track, 14 px labels. `sm`: 28 px track, 12 px labels. */
  readonly size?: "sm" | "md";
  readonly disabled?: boolean;
  readonly className?: string;
}

/** Position of the sliding indicator, in pixels relative to the track. */
interface IndicatorBox {
  readonly left: number;
  readonly width: number;
}

/**
 * A segmented control for one choice out of a few (Low/Medium/High,
 * Before/After/Split). A pill on the panel colour slides behind the
 * active segment (200 ms ease-out; instant with reduced motion). Exactly
 * one option is always selected: clicking the active one keeps it. Arrow
 * keys move the focus between segments.
 */
export function ToggleGroup<T extends string>({
  options,
  value,
  onValueChange,
  size = "md",
  disabled,
  className,
  "aria-label": ariaLabel,
}: ToggleGroupProps<T>) {
  const trackRef = useRef<HTMLDivElement>(null);
  const [indicator, setIndicator] = useState<IndicatorBox | null>(null);

  // Measure the active segment after layout and whenever the track
  // resizes (window resize, interface font size change, new labels).
  useLayoutEffect(() => {
    const track = trackRef.current;
    if (!track) return undefined;
    const measure = () => {
      const active = track.querySelector<HTMLElement>(`[data-value="${CSS.escape(value)}"]`);
      setIndicator(active ? { left: active.offsetLeft, width: active.offsetWidth } : null);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(track);
    return () => observer.disconnect();
  }, [value]);

  return (
    <ToggleGroupPrimitive.Root
      ref={trackRef}
      type="single"
      value={value}
      onValueChange={(next: string) => {
        const option = options.find((candidate) => candidate.value === next);
        if (option) onValueChange(option.value);
      }}
      disabled={disabled ?? false}
      aria-label={ariaLabel}
      data-slot="toggle-group"
      data-size={size}
      className={cn(
        "group/toggle relative inline-flex w-fit items-center gap-0.5 rounded-full bg-surface p-0.5 data-[size=md]:h-8 data-[size=sm]:h-7",
        disabled && "opacity-50",
        className,
      )}
    >
      {indicator && (
        <span
          aria-hidden
          data-slot="toggle-group-indicator"
          className="absolute inset-y-0.5 left-0 rounded-full bg-background transition-[translate,width] duration-200 ease-out"
          style={{ width: indicator.width, translate: `${indicator.left}px 0` }}
        />
      )}
      {options.map((option) => (
        <ToggleGroupPrimitive.Item
          key={option.value}
          value={option.value}
          disabled={option.disabled ?? false}
          data-value={option.value}
          data-slot="toggle-group-item"
          className="relative z-10 inline-flex h-full min-w-0 cursor-default items-center justify-center gap-1 rounded-full font-medium whitespace-nowrap text-foreground-subtle transition-colors group-data-[size=md]/toggle:px-2.5 group-data-[size=md]/toggle:text-ui-base group-data-[size=sm]/toggle:pr-2 group-data-[size=sm]/toggle:pl-1.5 group-data-[size=sm]/toggle:text-ui-sm hover:text-foreground disabled:pointer-events-none disabled:opacity-50 data-[state=on]:text-foreground [&_svg]:pointer-events-none [&_svg]:shrink-0 group-data-[size=md]/toggle:[&_svg]:size-3.5 group-data-[size=sm]/toggle:[&_svg]:size-3"
        >
          {option.icon}
          {option.label}
        </ToggleGroupPrimitive.Item>
      ))}
    </ToggleGroupPrimitive.Root>
  );
}
