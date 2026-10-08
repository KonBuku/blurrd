import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * A bordered card on the card colour: the first rounded container level
 * (12 px radius, 16 px padding, no shadow).
 */
export function Card({ className, ...props }: ComponentProps<"section">) {
  return (
    <section
      data-slot="card"
      className={cn(
        "flex flex-col gap-4 rounded-xl border border-card-border bg-card py-4 text-foreground",
        className,
      )}
      {...props}
    />
  );
}

/** The title row of a card. */
export function CardHeader({ className, ...props }: ComponentProps<"header">) {
  return (
    <header
      data-slot="card-header"
      className={cn("flex flex-col gap-1 px-4", className)}
      {...props}
    />
  );
}

/** The card title: base size, medium weight. */
export function CardTitle({ className, children, ...props }: ComponentProps<"h3">) {
  return (
    <h3 data-slot="card-title" className={cn("text-ui-base font-medium", className)} {...props}>
      {children}
    </h3>
  );
}

/** Secondary text below the card title. */
export function CardDescription({ className, ...props }: ComponentProps<"p">) {
  return (
    <p
      data-slot="card-description"
      className={cn("text-ui-sm text-foreground-subtle", className)}
      {...props}
    />
  );
}

/** The card body. */
export function CardContent({ className, ...props }: ComponentProps<"div">) {
  return <div data-slot="card-content" className={cn("px-4", className)} {...props} />;
}

/** Actions at the bottom of a card, right-aligned. */
export function CardFooter({ className, ...props }: ComponentProps<"footer">) {
  return (
    <footer
      data-slot="card-footer"
      className={cn("flex items-center justify-end gap-2 px-4", className)}
      {...props}
    />
  );
}
