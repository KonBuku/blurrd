import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * A content frame inset into the window shell: panel colour, 1 px
 * hairline and the 5 px radius of Windows 11 window content.
 */
export function Frame({ className, ...props }: ComponentProps<"section">) {
  return (
    <section
      data-slot="frame"
      className={cn(
        "flex min-h-0 min-w-0 flex-col overflow-hidden rounded-[5px] border border-border bg-background",
        className,
      )}
      {...props}
    />
  );
}

/**
 * The 40 px header row of a frame: title on the left, icon actions on the
 * right, a hairline below.
 */
export function FrameHeader({ className, ...props }: ComponentProps<"div">) {
  return (
    <div
      data-slot="frame-header"
      className={cn(
        "flex h-10 shrink-0 items-center gap-2 border-b border-border pr-1.5 pl-3",
        className,
      )}
      {...props}
    />
  );
}

/** The title of a frame header. */
export function FrameTitle({ className, children, ...props }: ComponentProps<"h2">) {
  return (
    <h2
      data-slot="frame-title"
      className={cn("min-w-0 truncate text-ui-base font-medium", className)}
      {...props}
    >
      {children}
    </h2>
  );
}
