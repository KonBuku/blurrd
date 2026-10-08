import { Switch as SwitchPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

export type SwitchProps = ComponentProps<typeof SwitchPrimitive.Root> & {
  /** `md` is 32 x 18 px, `sm` 28 x 16 px. */
  size?: "sm" | "md";
};

/**
 * An on/off switch. On is the primary colour, off the primary colour at
 * 30 %. An invisible area around the track enlarges the hit target.
 */
export function Switch({ className, size = "md", ...props }: SwitchProps) {
  return (
    <SwitchPrimitive.Root
      data-slot="switch"
      data-size={size}
      className={cn(
        "group/switch peer relative inline-flex shrink-0 items-center rounded-full p-px transition-colors after:absolute after:-inset-x-3 after:-inset-y-2 data-[size=md]:h-[18px] data-[size=md]:w-8 data-[size=sm]:h-4 data-[size=sm]:w-7 data-[state=checked]:bg-primary data-[state=unchecked]:bg-primary/30 data-disabled:cursor-not-allowed data-disabled:opacity-50",
        className,
      )}
      {...props}
    >
      <SwitchPrimitive.Thumb
        data-slot="switch-thumb"
        className="pointer-events-none block rounded-full bg-primary-foreground transition-transform group-data-[size=md]/switch:size-4 group-data-[size=sm]/switch:size-3.5 data-[state=checked]:translate-x-[calc(100%-2px)] data-[state=unchecked]:translate-x-0"
      />
    </SwitchPrimitive.Root>
  );
}
