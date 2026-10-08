import { X } from "lucide-react";
import { Dialog as SheetPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** A panel that slides over the window from the right edge. */
export const Sheet = SheetPrimitive.Root;

/** Closes the sheet; wrap a button with `asChild`. */
export const SheetClose = SheetPrimitive.Close;

/** The sheet title, announced when it opens. */
export function SheetTitle({ className, ...props }: ComponentProps<typeof SheetPrimitive.Title>) {
  return (
    <SheetPrimitive.Title
      data-slot="sheet-title"
      className={cn("min-w-0 truncate text-ui-base font-medium text-foreground", className)}
      {...props}
    />
  );
}

/** Secondary text below the title. */
export function SheetDescription({
  className,
  ...props
}: ComponentProps<typeof SheetPrimitive.Description>) {
  return (
    <SheetPrimitive.Description
      data-slot="sheet-description"
      className={cn("text-ui-sm text-foreground-subtle", className)}
      {...props}
    />
  );
}

/**
 * The sheet panel: a frame (12 px radius, hairline, large shadow) 4 px
 * inside the right window edge, on a light backdrop. It slides in from the
 * right over 200 ms (fade only with reduced motion). Focus is trapped
 * inside and Escape closes it.
 */
export function SheetContent({
  className,
  children,
  showClose = true,
  ...props
}: ComponentProps<typeof SheetPrimitive.Content> & {
  /** Show the close button in the top-right corner. */
  showClose?: boolean;
}) {
  return (
    <SheetPrimitive.Portal>
      <SheetPrimitive.Overlay
        data-slot="sheet-overlay"
        className="fixed inset-0 z-50 bg-black/30 duration-200 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=open]:animate-in data-[state=open]:fade-in-0"
      />
      <SheetPrimitive.Content
        data-slot="sheet-content"
        className={cn(
          "fixed inset-y-1 right-1 z-50 flex max-w-[calc(100%-0.5rem)] flex-col overflow-hidden rounded-xl border border-popover-border bg-background text-foreground shadow-lg duration-200 ease-out outline-none data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:slide-out-to-right-8 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:slide-in-from-right-8",
          className,
        )}
        {...props}
      >
        {children}
        {showClose && (
          <SheetPrimitive.Close
            data-slot="sheet-close"
            aria-label="Close"
            className="absolute top-2 right-2 grid size-6 place-items-center rounded-md text-foreground-subtle transition-colors hover:bg-surface-hover hover:text-foreground"
          >
            <X aria-hidden className="size-4" />
          </SheetPrimitive.Close>
        )}
      </SheetPrimitive.Content>
    </SheetPrimitive.Portal>
  );
}
