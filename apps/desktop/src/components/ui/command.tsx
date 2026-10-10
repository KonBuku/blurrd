import { Command as CommandPrimitive } from "cmdk";
import { Search } from "lucide-react";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/**
 * A filterable command list (cmdk): a search field on top of grouped
 * items. Arrow keys move the highlighted item, Enter runs it, typing
 * filters by label and keywords.
 */
export function Command({ className, ...props }: ComponentProps<typeof CommandPrimitive>) {
  return (
    <CommandPrimitive
      data-slot="command"
      className={cn(
        "flex h-full w-full flex-col overflow-hidden text-popover-foreground",
        className,
      )}
      {...props}
    />
  );
}

/** The search field with a leading magnifier, separated from the list by a rule. */
export function CommandInput({
  className,
  ...props
}: ComponentProps<typeof CommandPrimitive.Input>) {
  return (
    <div
      data-slot="command-input-wrapper"
      className="flex h-11 shrink-0 items-center gap-2 border-b border-border px-3"
    >
      <Search aria-hidden className="size-4 shrink-0 text-foreground-subtle" />
      <CommandPrimitive.Input
        data-slot="command-input"
        className={cn(
          "h-full min-w-0 flex-1 bg-transparent text-ui-base text-foreground outline-none placeholder:text-foreground-subtlest disabled:cursor-not-allowed disabled:opacity-50",
          className,
        )}
        {...props}
      />
    </div>
  );
}

/** The scrolling list of groups and items. */
export function CommandList({ className, ...props }: ComponentProps<typeof CommandPrimitive.List>) {
  return (
    <CommandPrimitive.List
      data-slot="command-list"
      className={cn(
        "max-h-90 scroll-py-1 overflow-x-hidden overflow-y-auto overscroll-contain p-1",
        className,
      )}
      {...props}
    />
  );
}

/** Shown in place of the list when nothing matches the search. */
export function CommandEmpty({
  className,
  ...props
}: ComponentProps<typeof CommandPrimitive.Empty>) {
  return (
    <CommandPrimitive.Empty
      data-slot="command-empty"
      className={cn("py-6 text-center text-ui-base text-foreground-subtle", className)}
      {...props}
    />
  );
}

/** A group of items under a small heading. */
export function CommandGroup({
  className,
  ...props
}: ComponentProps<typeof CommandPrimitive.Group>) {
  return (
    <CommandPrimitive.Group
      data-slot="command-group"
      className={cn(
        "overflow-hidden not-first:mt-1 [&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:pt-1.5 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:text-ui-xs [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:text-foreground-subtle",
        className,
      )}
      {...props}
    />
  );
}

/**
 * One command: optional leading icon, label, trailing hint or shortcut.
 * The highlighted item uses the menu hover colour; disabled items stay
 * listed (so the search finds them) but cannot run.
 */
export function CommandItem({ className, ...props }: ComponentProps<typeof CommandPrimitive.Item>) {
  return (
    <CommandPrimitive.Item
      data-slot="command-item"
      className={cn(
        "relative flex min-h-8 cursor-default items-center gap-2 rounded-lg px-2 py-1 text-ui-base/relaxed outline-none select-none data-[disabled=true]:pointer-events-none data-[disabled=true]:text-foreground-subtlest data-[selected=true]:bg-menu-hover [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4",
        className,
      )}
      {...props}
    />
  );
}

/** The right-aligned hint (state or shortcut) of an item. */
export function CommandShortcut({ className, ...props }: ComponentProps<"span">) {
  return (
    <span
      data-slot="command-shortcut"
      className={cn("ml-auto flex shrink-0 items-center gap-1 pl-4", className)}
      {...props}
    />
  );
}
