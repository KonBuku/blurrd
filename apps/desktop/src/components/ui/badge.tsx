import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** Pill badge styles: 20 px tall, 10 px medium text. */
export const badgeVariants = cva(
  "inline-flex h-5 w-fit shrink-0 items-center justify-center gap-1 overflow-hidden rounded-full border border-transparent px-2 text-ui-xs font-medium whitespace-nowrap [&>svg]:pointer-events-none [&>svg]:size-2.5",
  {
    variants: {
      variant: {
        primary: "bg-primary text-primary-foreground",
        secondary: "bg-secondary text-secondary-foreground",
        outline: "border-border text-foreground",
        success: "bg-success/14 text-success dark:bg-success/18",
        warning: "bg-warning/14 text-warning dark:bg-warning/18",
        destructive: "bg-destructive/10 text-destructive dark:bg-destructive/20",
      },
    },
    defaultVariants: { variant: "secondary" },
  },
);

export type BadgeProps = ComponentProps<"span"> & VariantProps<typeof badgeVariants>;

/** A small status or count label. */
export function Badge({ className, variant, ...props }: BadgeProps) {
  return (
    <span data-slot="badge" className={cn(badgeVariants({ variant }), className)} {...props} />
  );
}
