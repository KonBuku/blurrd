import { Check } from "lucide-react";
import { DropdownMenu as MenuPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** A dropdown menu: wraps a trigger and its content. */
export const DropdownMenu = MenuPrimitive.Root;

/** The element that opens the menu. */
export const DropdownMenuTrigger = MenuPrimitive.Trigger;

/** A labelled group of items. */
export const DropdownMenuGroup = MenuPrimitive.Group;

/** A group of {@link DropdownMenuRadioItem}s, one of which is selected. */
export const DropdownMenuRadioGroup = MenuPrimitive.RadioGroup;

/**
 * Menu panel styles shared with the context menu: menu colour, 8 px
 * radius, 4 px padding, shadow, 100 ms fade and zoom.
 */
export const menuContentClass =
  "z-50 flex min-w-32 flex-col gap-0.5 overflow-x-hidden overflow-y-auto rounded-lg border border-popover-border bg-menu p-1 text-popover-foreground shadow-md duration-100 data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2 data-[state=closed]:animate-out data-[state=closed]:fade-out-0 data-[state=closed]:zoom-out-95 data-[state=open]:animate-in data-[state=open]:fade-in-0 data-[state=open]:zoom-in-95";

/** The menu panel: menu colour, 8 px radius, 4 px padding, shadow. */
export function DropdownMenuContent({
  className,
  sideOffset = 2,
  ...props
}: ComponentProps<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        data-slot="dropdown-menu-content"
        sideOffset={sideOffset}
        className={cn(
          menuContentClass,
          "max-h-(--radix-dropdown-menu-content-available-height) origin-(--radix-dropdown-menu-content-transform-origin)",
          className,
        )}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

const itemClass =
  "relative flex min-h-7 cursor-default items-center gap-2 rounded-md px-2 py-1 text-ui-base/relaxed outline-none select-none data-disabled:pointer-events-none data-disabled:text-foreground-subtlest data-highlighted:bg-menu-hover [&_svg]:pointer-events-none [&_svg]:shrink-0 [&_svg:not([class*='size-'])]:size-4";

/**
 * Command item styles shared with the context menu; destructive items
 * (`data-variant="destructive"`) use the destructive colour.
 */
export const menuItemClass = `${itemClass} data-[variant=destructive]:text-destructive data-[variant=destructive]:data-highlighted:bg-destructive data-[variant=destructive]:data-highlighted:text-destructive-foreground`;

export type DropdownMenuItemProps = ComponentProps<typeof MenuPrimitive.Item> & {
  /** Destructive items use the destructive colour. */
  variant?: "default" | "destructive";
};

/** A menu command with an optional leading icon and trailing shortcut. */
export function DropdownMenuItem({
  className,
  variant = "default",
  ...props
}: DropdownMenuItemProps) {
  return (
    <MenuPrimitive.Item
      data-slot="dropdown-menu-item"
      data-variant={variant}
      className={cn(menuItemClass, className)}
      {...props}
    />
  );
}

/** A menu item that toggles a setting; the check sits on the right. */
export function DropdownMenuCheckboxItem({
  className,
  children,
  ...props
}: ComponentProps<typeof MenuPrimitive.CheckboxItem>) {
  return (
    <MenuPrimitive.CheckboxItem
      data-slot="dropdown-menu-checkbox-item"
      className={cn(itemClass, "py-1.5 pr-8", className)}
      {...props}
    >
      {children}
      <MenuPrimitive.ItemIndicator className="absolute right-2 grid place-items-center">
        <Check aria-hidden className="text-foreground-subtle" />
      </MenuPrimitive.ItemIndicator>
    </MenuPrimitive.CheckboxItem>
  );
}

/** One choice of a radio group; the selected one shows a check on the right. */
export function DropdownMenuRadioItem({
  className,
  children,
  ...props
}: ComponentProps<typeof MenuPrimitive.RadioItem>) {
  return (
    <MenuPrimitive.RadioItem
      data-slot="dropdown-menu-radio-item"
      className={cn(itemClass, "py-1.5 pr-8", className)}
      {...props}
    >
      {children}
      <MenuPrimitive.ItemIndicator className="absolute right-2 grid place-items-center">
        <Check aria-hidden className="text-foreground-subtle" />
      </MenuPrimitive.ItemIndicator>
    </MenuPrimitive.RadioItem>
  );
}

/** A group heading inside the menu. */
export function DropdownMenuLabel({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Label>) {
  return (
    <MenuPrimitive.Label
      data-slot="dropdown-menu-label"
      className={cn("px-2 py-1.5 text-ui-base font-medium text-foreground-subtlest", className)}
      {...props}
    />
  );
}

/** A full-width divider between groups. */
export function DropdownMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Separator>) {
  return (
    <MenuPrimitive.Separator
      data-slot="dropdown-menu-separator"
      className={cn("-mx-1 my-1 h-px bg-border", className)}
      {...props}
    />
  );
}

/** The right-aligned shortcut hint of a menu item. */
export function DropdownMenuShortcut({ className, ...props }: ComponentProps<"span">) {
  return (
    <span
      data-slot="dropdown-menu-shortcut"
      className={cn(
        "ml-auto pl-4 text-ui-xs tracking-widest text-foreground-subtlest in-data-highlighted:text-foreground-subtle",
        className,
      )}
      {...props}
    />
  );
}
