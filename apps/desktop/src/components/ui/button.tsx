import { cva, type VariantProps } from "class-variance-authority";
import { Slot } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * Button styles. Only colours transition (never layout properties), the
 * default size is 28 px, and icon-only sizes are square. Exported so that
 * links and triggers can look like buttons.
 */
export const buttonVariants = cva(
  "inline-flex shrink-0 cursor-default items-center justify-center rounded-md border border-transparent text-ui-base/relaxed whitespace-nowrap transition-colors select-none disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive [&_svg]:pointer-events-none [&_svg]:shrink-0",
  {
    variants: {
      variant: {
        primary: "bg-primary text-primary-foreground hover:bg-primary/80",
        secondary:
          "bg-secondary text-secondary-foreground hover:bg-secondary/80 aria-expanded:bg-secondary",
        outline:
          "border-border text-foreground hover:border-border-hover hover:bg-input/50 aria-expanded:bg-input/50",
        ghost: "text-foreground hover:bg-hover aria-expanded:bg-hover",
        destructive:
          "bg-destructive text-destructive-foreground hover:bg-destructive/90 aria-expanded:bg-destructive",
        warning: "bg-warning text-warning-foreground hover:bg-warning/90",
        link: "text-link underline-offset-4 hover:underline",
      },
      size: {
        xs: "h-5 gap-1 rounded-sm px-2 has-[>svg:first-child]:pl-1.5 [&_svg:not([class*='size-'])]:size-2.5",
        sm: "h-6 gap-1 px-2 has-[>svg:first-child]:pl-1.5 [&_svg:not([class*='size-'])]:size-3",
        md: "h-7 gap-1 px-2 has-[>svg:first-child]:pl-1.5 [&_svg:not([class*='size-'])]:size-3.5",
        lg: "h-8 gap-1 rounded-lg px-2.5 has-[>svg:first-child]:pl-2 [&_svg:not([class*='size-'])]:size-4",
        "icon-xs": "size-5 rounded-sm [&_svg:not([class*='size-'])]:size-2.5",
        "icon-sm": "size-6 [&_svg:not([class*='size-'])]:size-3",
        "icon-md": "size-7 rounded-lg [&_svg:not([class*='size-'])]:size-4",
        "icon-lg": "size-8 rounded-lg [&_svg:not([class*='size-'])]:size-4",
      },
    },
    defaultVariants: { variant: "primary", size: "md" },
  },
);

export type ButtonProps = ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & {
    /** Render the single child element with the button styles instead. */
    asChild?: boolean;
  };

/**
 * A push button. Defaults to `type="button"`, so it never submits a form
 * by accident.
 */
export function Button({
  className,
  variant,
  size,
  asChild = false,
  type = "button",
  ...props
}: ButtonProps) {
  if (asChild) {
    return (
      <Slot.Root
        data-slot="button"
        className={cn(buttonVariants({ variant, size }), className)}
        {...props}
      />
    );
  }
  return (
    <button
      data-slot="button"
      type={type}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  );
}
