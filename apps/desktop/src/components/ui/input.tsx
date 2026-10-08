import { cva, type VariantProps } from "class-variance-authority";
import type { ComponentProps } from "react";

import { cn } from "@/lib/utils";

/** Text field styles; focus darkens the border instead of adding a glow. */
export const inputVariants = cva(
  "w-full min-w-0 border border-input-border bg-input text-ui-base/relaxed text-foreground transition-colors placeholder:text-foreground-subtlest hover:border-input-border-hover focus-visible:border-input-border-focused focus-visible:bg-input-focused disabled:pointer-events-none disabled:opacity-50 aria-invalid:border-destructive aria-invalid:ring-2 aria-invalid:ring-destructive/20 dark:aria-invalid:ring-destructive/40",
  {
    variants: {
      size: {
        xs: "h-5 rounded-sm px-2",
        sm: "h-6 rounded-md px-2",
        md: "h-7 rounded-md px-2",
        lg: "h-8 rounded-lg px-3",
      },
    },
    defaultVariants: { size: "md" },
  },
);

export type InputProps = Omit<ComponentProps<"input">, "size"> &
  VariantProps<typeof inputVariants> & {
    /** The native `size` attribute (visible width in characters). */
    htmlSize?: number;
  };

/** A single-line text field. */
export function Input({ className, size, htmlSize, type = "text", ...props }: InputProps) {
  return (
    <input
      data-slot="input"
      type={type}
      size={htmlSize}
      className={cn(inputVariants({ size }), className)}
      {...props}
    />
  );
}
