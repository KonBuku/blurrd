import type { ReactNode } from "react";

import { cn } from "@/lib/utils";
import { useSettingsStore } from "@/store/settings";

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
      className="@container/settings overflow-hidden rounded-xl border border-card-border bg-card"
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
 * the first. In a narrow group (small window, large interface size) the
 * control moves below the label instead of squeezing it.
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
        "grid grid-cols-1 items-center gap-x-4 gap-y-2 border-t border-border px-4 py-3 first:border-t-0",
        controlWidth === "wide"
          ? "@[36rem]/settings:grid-cols-[minmax(0,1fr)_280px]"
          : "@[30rem]/settings:grid-cols-[minmax(0,1fr)_192px]",
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
      <div
        className={cn(
          "flex min-w-0 flex-wrap items-center justify-start gap-2",
          controlWidth === "wide"
            ? "@[36rem]/settings:justify-end"
            : "@[30rem]/settings:justify-end",
        )}
      >
        {control}
      </div>
    </div>
  );
}

/** The notice a page shows while the settings file cannot be written. */
export function ReadOnlyNotice() {
  const status = useSettingsStore((state) => state.status);
  if (status !== "read-only") return null;
  return (
    <p className="text-ui-base text-warning">
      The settings file cannot be written. Changes apply until blurrd closes.
    </p>
  );
}
