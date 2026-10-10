import type { VerifySummary } from "@/bindings/VerifySummary";
import { Badge } from "@/components/ui/badge";
import { stateLabel } from "@/lib/verify-report";
import { cn } from "@/lib/utils";

// Full class names, so that Tailwind finds them in the source.
const DOT = {
  success: "bg-success",
  warning: "bg-warning",
  destructive: "bg-destructive",
} as const;

/**
 * The result of a verification as an outlined badge: the text in the
 * foreground colour (readable on cards and dialogs alike) and a dot in
 * the result's colour.
 */
export function ResultBadge({ summary }: { readonly summary: VerifySummary }) {
  const { text, tone } = stateLabel(summary);
  return (
    <Badge variant="outline" data-tone={tone}>
      <span aria-hidden className={cn("size-1.5 rounded-full", DOT[tone])} />
      {text}
    </Badge>
  );
}
