import { ContextMenu as MenuPrimitive } from "radix-ui";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

import { menuContentClass, menuItemClass } from "./dropdown-menu";

/** A right-click menu: wraps the area that opens it and its content. */
export const ContextMenu = MenuPrimitive.Root;

/** The area that opens the menu on right-click or the context menu key. */
export const ContextMenuTrigger = MenuPrimitive.Trigger;

/** The menu panel, styled like the dropdown menu, at the pointer. */
export function ContextMenuContent({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Content>) {
  return (
    <MenuPrimitive.Portal>
      <MenuPrimitive.Content
        data-slot="context-menu-content"
        className={cn(
          menuContentClass,
          "max-h-(--radix-context-menu-content-available-height) origin-(--radix-context-menu-content-transform-origin)",
          className,
        )}
        {...props}
      />
    </MenuPrimitive.Portal>
  );
}

export type ContextMenuItemProps = ComponentProps<typeof MenuPrimitive.Item> & {
  /** Destructive items use the destructive colour. */
  variant?: "default" | "destructive";
};

/** A menu command with an optional leading icon and trailing shortcut. */
export function ContextMenuItem({
  className,
  variant = "default",
  ...props
}: ContextMenuItemProps) {
  return (
    <MenuPrimitive.Item
      data-slot="context-menu-item"
      data-variant={variant}
      className={cn(menuItemClass, className)}
      {...props}
    />
  );
}

/** A full-width divider between groups. */
export function ContextMenuSeparator({
  className,
  ...props
}: ComponentProps<typeof MenuPrimitive.Separator>) {
  return (
    <MenuPrimitive.Separator
      data-slot="context-menu-separator"
      className={cn("-mx-1 my-1 h-px bg-border", className)}
      {...props}
    />
  );
}
