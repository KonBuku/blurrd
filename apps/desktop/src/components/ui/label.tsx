import { Label as LabelPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** A form label in medium weight; clicking it focuses its control. */
export function Label({ className, ...props }: ComponentProps<typeof LabelPrimitive.Root>) {
  return (
    <LabelPrimitive.Root
      data-slot="label"
      className={cn(
        "flex items-center gap-2 text-ui-base font-medium text-foreground select-none peer-disabled:opacity-50",
        className,
      )}
      {...props}
    />
  );
}
