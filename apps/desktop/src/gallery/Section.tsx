import type { ReactNode } from "react";

import { cn } from "@/lib/utils";

/** A titled block of the component gallery. */
export function Section({
  title,
  children,
  className,
}: {
  readonly title: string;
  readonly children: ReactNode;
  readonly className?: string;
}) {
  const id = `gallery-${title.toLowerCase().replaceAll(/[^a-z]+/g, "-")}`;
  return (
    <section aria-labelledby={id} className="flex flex-col gap-3">
      <h2 id={id} className="text-ui-sm font-medium text-foreground-subtlest">
        {title}
      </h2>
      <div className={cn("flex flex-wrap items-center gap-3", className)}>{children}</div>
    </section>
  );
}

/** A captioned cell inside a section. */
export function Specimen({
  label,
  children,
}: {
  readonly label: string;
  readonly children: ReactNode;
}) {
  return (
    <div className="flex flex-col items-start gap-1.5">
      {children}
      <span className="font-mono text-ui-xs text-foreground-subtlest">{label}</span>
    </div>
  );
}
