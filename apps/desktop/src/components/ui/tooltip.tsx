import { Tooltip as TooltipPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * A tooltip: wraps one trigger and its content. Tooltips appear without
 * delay; each one carries its own provider, so no app-wide setup is
 * needed.
 */
export function Tooltip({
  delayDuration = 0,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Root>) {
  return (
    <TooltipPrimitive.Provider delayDuration={delayDuration}>
      <TooltipPrimitive.Root {...props} />
    </TooltipPrimitive.Provider>
  );
}

/** The element that shows the tooltip on hover and keyboard focus. */
export const TooltipTrigger = TooltipPrimitive.Trigger;

/** The floating label: 12 px text on the tooltip colour, 8 px radius. */
export function TooltipContent({
  className,
  sideOffset = 4,
  children,
  ...props
}: ComponentProps<typeof TooltipPrimitive.Content>) {
  return (
    <TooltipPrimitive.Portal>
      <TooltipPrimitive.Content
        data-slot="tooltip-content"
        sideOffset={sideOffset}
        className={cn(
          "z-50 inline-flex w-fit max-w-80 origin-(--radix-tooltip-content-transform-origin) items-center gap-2 rounded-lg border border-border bg-tooltip px-3 py-1.5 text-ui-sm text-tooltip-foreground duration-100 animate-in fade-in-0 zoom-in-95 has-data-[slot=kbd]:pr-1.5 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95",
          className,
        )}
        {...props}
      >
        {children}
      </TooltipPrimitive.Content>
    </TooltipPrimitive.Portal>
  );
}
