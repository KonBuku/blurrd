import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

export interface SettingsPageProps {
  /** Page title, also the accessible name of the page region. */
  readonly title: string;
  /** One sentence below the title. */
  readonly description?: string;
  readonly children: ReactNode;
}

/** One settings page: title, description and its groups, 32 px apart. */
export function SettingsPage({ title, description, children }: SettingsPageProps) {
  return (
    <div className="flex flex-col gap-8">
      <div className="flex flex-col gap-1">
        <h1 className="text-ui-xl font-semibold">{title}</h1>
        {description && <p className="text-ui-base text-foreground-subtle">{description}</p>}
      </div>
      {children}
    </div>
  );
}

/** A card of settings rows separated by hairlines. */
export function SettingsGroup({ children }: { readonly children: ReactNode }) {
  return (
    <div
      data-slot="settings-group"
      className="overflow-hidden rounded-xl border border-card-border bg-card"
    >
      {children}
    </div>
  );
}

export interface SettingsRowProps {
  /** Row label; give it `labelId` and point the control at it. */
  readonly label: ReactNode;
  /** Id of the label element, for the control's `aria-labelledby`. */
  readonly labelId?: string;
  /** Explanation below the label. */
  readonly description?: ReactNode;
  /** Controls, right-aligned. */
  readonly control: ReactNode;
  /** `wide` gives the control column 280 px instead of 192 px. */
  readonly controlWidth?: "default" | "wide";
}

/**
 * One setting: label and description on the left, the control in a fixed
 * right-aligned column, 12 x 16 px padding, a hairline above every row but
 * the first.
 */
export function SettingsRow({
  label,
  labelId,
  description,
  control,
  controlWidth = "default",
}: SettingsRowProps) {
  return (
    <div
      data-slot="settings-row"
      className={cn(
        "grid items-center gap-4 border-t border-border px-4 py-3 first:border-t-0",
        controlWidth === "wide"
          ? "grid-cols-[minmax(0,1fr)_280px]"
          : "grid-cols-[minmax(0,1fr)_192px]",
      )}
    >
      <div className="min-w-0">
        <div id={labelId} className="text-ui-base font-medium">
          {label}
        </div>
        {description && (
          <div className="mt-1 text-ui-base leading-6 text-foreground-subtle">{description}</div>
        )}
      </div>
      <div className="flex min-w-0 items-center justify-end gap-2">{control}</div>
    </div>
  );
}
