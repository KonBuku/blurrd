import { Check, ChevronDown } from "lucide-react";
import { Select as SelectPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** A select: wraps a trigger and its option panel. */
export const Select = SelectPrimitive.Root;

/** A labelled group of options. */
export const SelectGroup = SelectPrimitive.Group;

/** Shows the selected option, or the placeholder, inside the trigger. */
export const SelectValue = SelectPrimitive.Value;

export type SelectTriggerProps = ComponentProps<typeof SelectPrimitive.Trigger> & {
  /** Matches the text field heights: 24, 28 or 32 px. */
  size?: "sm" | "md" | "lg";
};

/** The closed select, styled like a text field with a chevron. */
export function SelectTrigger({ className, size = "md", children, ...props }: SelectTriggerProps) {
  return (
    <SelectPrimitive.Trigger
      data-slot="select-trigger"
      data-size={size}
      className={cn(
        "flex w-fit min-w-0 items-center justify-between gap-1.5 rounded-md border border-input-border bg-input pr-1 pl-2 text-ui-base/relaxed whitespace-nowrap text-foreground transition-colors hover:border-input-border-hover data-disabled:pointer-events-none data-disabled:opacity-50 data-placeholder:text-foreground-subtlest data-[size=lg]:h-8 data-[size=lg]:rounded-lg data-[size=lg]:pr-2 data-[size=lg]:pl-3 data-[size=md]:h-7 data-[size=sm]:h-6 data-[state=open]:border-input-border-focused *:data-[slot=select-value]:truncate",
        className,
      )}
      {...props}
    >
      {children}
      <SelectPrimitive.Icon asChild>
        <ChevronDown aria-hidden className="size-3.5 shrink-0 text-foreground-subtle" />
      </SelectPrimitive.Icon>
    </SelectPrimitive.Trigger>
  );
}

/** The option panel: menu colour, 8 px radius, 4 px padding, shadow. */
export function SelectContent({
  className,
  children,
  position = "popper",
  ...props
}: ComponentProps<typeof SelectPrimitive.Content>) {
  return (
    <SelectPrimitive.Portal>
      <SelectPrimitive.Content
        data-slot="select-content"
        position={position}
        className={cn(
          "relative z-50 max-h-(--radix-select-content-available-height) min-w-32 origin-(--radix-select-content-transform-origin) overflow-x-hidden overflow-y-auto rounded-lg border border-popover-border bg-menu text-popover-foreground shadow-md duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=top]:slide-in-from-bottom-2 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95",
          position === "popper" &&
            "w-full min-w-(--radix-select-trigger-width) data-[side=bottom]:translate-y-0.5 data-[side=top]:-translate-y-0.5",
          className,
        )}
        {...props}
      >
        <SelectPrimitive.Viewport className="flex flex-col gap-0.5 p-1">
          {children}
        </SelectPrimitive.Viewport>
      </SelectPrimitive.Content>
    </SelectPrimitive.Portal>
  );
}

/** A group heading inside the panel. */
export function SelectLabel({ className, ...props }: ComponentProps<typeof SelectPrimitive.Label>) {
  return (
    <SelectPrimitive.Label
      data-slot="select-label"
      className={cn("px-2 py-1.5 text-ui-base text-foreground-subtlest", className)}
      {...props}
    />
  );
}

/** One option; the selected one shows a trailing check. */
export function SelectItem({
  className,
  children,
  ...props
}: ComponentProps<typeof SelectPrimitive.Item>) {
  return (
    <SelectPrimitive.Item
      data-slot="select-item"
      className={cn(
        "relative flex min-h-7 w-full items-center gap-2 rounded-md py-1 pr-8 pl-2 text-ui-base/relaxed outline-none select-none data-disabled:pointer-events-none data-disabled:text-foreground-subtlest data-highlighted:bg-menu-hover",
        className,
      )}
      {...props}
    >
      <SelectPrimitive.ItemText>{children}</SelectPrimitive.ItemText>
      <SelectPrimitive.ItemIndicator className="absolute right-2 grid place-items-center">
        <Check aria-hidden className="size-4 text-foreground-subtle" />
      </SelectPrimitive.ItemIndicator>
    </SelectPrimitive.Item>
  );
}

/** A divider between option groups. */
export function SelectSeparator({
  className,
  ...props
}: ComponentProps<typeof SelectPrimitive.Separator>) {
  return (
    <SelectPrimitive.Separator
      data-slot="select-separator"
      className={cn("my-1 h-px bg-border", className)}
      {...props}
    />
  );
}
