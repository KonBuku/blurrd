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

/**
 * The title of a frame header: a level-two heading, or level one for the
 * page's main subject (the previewed recording).
 */
export function FrameTitle({
  className,
  children,
  level = 2,
  ...props
}: ComponentProps<"h2"> & { readonly level?: 1 | 2 }) {
  const Heading = level === 1 ? "h1" : "h2";
  return (
    <Heading
      data-slot="frame-title"
      className={cn("min-w-0 truncate text-ui-base font-medium", className)}
      {...props}
    >
      {children}
    </Heading>
  );
}
