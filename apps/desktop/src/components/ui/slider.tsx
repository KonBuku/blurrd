import { Slider as SliderPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export type SliderProps = ComponentProps<typeof SliderPrimitive.Root> & {
  /**
   * Value restored by a double-click on a thumb (for example the default
   * blur strength). Reported through `onValueChange`, so it needs a
   * controlled `value`.
   */
  resetValue?: number[];
};

/**
 * A range slider: 4 px track in the primary colour at 20 % with the filled
 * part in the primary colour, and a 16 px round thumb on the card colour.
 * `aria-label` / `aria-labelledby` name every thumb (the element that
 * takes focus); arrow keys step, Page Up/Down step by ten.
 */
export function Slider({
  className,
  resetValue,
  "aria-label": ariaLabel,
  "aria-labelledby": ariaLabelledBy,
  ...props
}: SliderProps) {
  const thumbs = props.value ?? props.defaultValue ?? [props.min ?? 0];
  const reset = resetValue === undefined ? undefined : () => props.onValueChange?.(resetValue);
  return (
    <SliderPrimitive.Root
      data-slot="slider"
      className={cn(
        "relative flex h-5 w-full touch-none items-center select-none data-disabled:opacity-50 data-[orientation=vertical]:h-full data-[orientation=vertical]:min-h-32 data-[orientation=vertical]:w-5 data-[orientation=vertical]:flex-col",
        className,
      )}
      {...props}
    >
      <SliderPrimitive.Track
        data-slot="slider-track"
        className="relative grow overflow-hidden rounded-full bg-primary/20 data-[orientation=horizontal]:h-1 data-[orientation=horizontal]:w-full data-[orientation=vertical]:h-full data-[orientation=vertical]:w-1"
      >
        <SliderPrimitive.Range
          data-slot="slider-range"
          className="absolute rounded-full bg-primary data-[orientation=horizontal]:h-full data-[orientation=vertical]:w-full"
        />
      </SliderPrimitive.Track>
      {thumbs.map((_, index) => (
        <SliderPrimitive.Thumb
          // Thumbs have no identity beyond their position.
          // oxlint-disable-next-line react/no-array-index-key
          key={index}
          data-slot="slider-thumb"
          aria-label={ariaLabel}
          aria-labelledby={ariaLabelledBy}
          onDoubleClick={reset}
          className="block size-4 rounded-full border border-border-hover bg-card shadow-md transition-colors hover:border-foreground-subtlest data-disabled:pointer-events-none"
        />
      ))}
    </SliderPrimitive.Root>
  );
}
