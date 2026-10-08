import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** A keyboard shortcut chip, for tooltips and menus. */
export function Kbd({ className, ...props }: ComponentProps<"kbd">) {
  return (
    <kbd
      data-slot="kbd"
      className={cn(
        "inline-flex h-5 min-w-5 items-center justify-center gap-1 rounded-sm bg-tag px-1 font-mono text-ui-xs font-medium text-tag-foreground",
        className,
      )}
      {...props}
    />
  );
}
