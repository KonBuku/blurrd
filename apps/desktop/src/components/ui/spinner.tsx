import { Loader } from "lucide-react";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export type SpinnerProps = ComponentProps<"output"> & {
  /** Accessible name announced while the spinner is visible. */
  label?: string;
};

/**
 * The loading indicator: a 16 px loader icon spinning linearly, inside a
 * status element that announces `label`.
 */
export function Spinner({ className, label = "Loading", ...props }: SpinnerProps) {
  return (
    <output
      data-slot="spinner"
      aria-label={label}
      className={cn("inline-flex size-4 shrink-0", className)}
      {...props}
    >
      <Loader aria-hidden className="size-full animate-spin" />
    </output>
  );
}
