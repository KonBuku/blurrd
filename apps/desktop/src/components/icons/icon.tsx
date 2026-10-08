import type { ReactNode, SVGProps } from "react";

import { cn } from "@/lib/utils";

/** Props of every blurrd icon. */
export type IconProps = Omit<SVGProps<SVGSVGElement>, "children"> & {
  /**
   * Accessible name. Without it the icon is decorative (`aria-hidden`),
   * which is right when a text label sits next to it.
   */
  title?: string;
};

/** A blurrd icon component, as returned by {@link createIcon}. */
export type Icon = ((props: IconProps) => ReactNode) & { displayName: string };

/**
 * Builds an icon on the shared grid: 24 x 24 view box, outline only,
 * 1.5 stroke with round caps and joins in the current text colour, 16 px
 * by default. This matches the Lucide icons used for generic actions, so
 * both sets can sit side by side.
 */
export function createIcon(displayName: string, shapes: ReactNode): Icon {
  function BlurrdIcon({ className, title, strokeWidth = 1.5, ...props }: IconProps) {
    return (
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 24 24"
        fill="none"
        stroke="currentColor"
        strokeWidth={strokeWidth}
        strokeLinecap="round"
        strokeLinejoin="round"
        role={title === undefined ? undefined : "img"}
        aria-hidden={title === undefined ? true : undefined}
        data-icon={displayName}
        className={cn("size-4 shrink-0", className)}
        {...props}
      >
        {title !== undefined && <title>{title}</title>}
        {shapes}
      </svg>
    );
  }
  BlurrdIcon.displayName = displayName;
  return BlurrdIcon;
}
