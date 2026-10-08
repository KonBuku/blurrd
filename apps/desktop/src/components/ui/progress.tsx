import { Progress as ProgressPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export type ProgressProps = Omit<ComponentProps<typeof ProgressPrimitive.Root>, "value"> & {
  /**
   * Completion from 0 to 100. `null` shows the indeterminate sweep, for
   * work whose length is not known yet.
   */
  value: number | null;
};

/**
 * A thin progress bar: 4 px track in the surface colour with an accent
 * indicator. The indeterminate sweep falls back to a full static bar when
 * the user prefers reduced motion.
 */
export function Progress({ className, value, max = 100, ...props }: ProgressProps) {
  const clamped = value === null ? null : Math.min(Math.max(value, 0), max);
  return (
    <ProgressPrimitive.Root
      data-slot="progress"
      value={clamped}
      max={max}
      className={cn("relative h-1 w-full overflow-hidden rounded-md bg-primary/10", className)}
      {...props}
    >
      {clamped === null ? (
        <ProgressPrimitive.Indicator
          data-slot="progress-indicator"
          className="absolute inset-y-0 w-1/3 animate-progress-sweep rounded-full bg-accent-strong motion-reduce:left-0 motion-reduce:w-full motion-reduce:animate-none motion-reduce:opacity-60"
        />
      ) : (
        <ProgressPrimitive.Indicator
          data-slot="progress-indicator"
          className="h-full rounded-full bg-accent-strong transition-[width] duration-200 ease-out"
          style={{ width: `${(clamped / max) * 100}%` }}
        />
      )}
    </ProgressPrimitive.Root>
  );
}
