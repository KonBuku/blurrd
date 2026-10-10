import {
  CircleCheck,
  CircleDot,
  CircleSlash,
  CircleX,
  Clock,
  Loader,
  ScanSearch,
  ShieldQuestionMark,
  TriangleAlert,
  type LucideIcon,
} from "lucide-react";

import {
  statusTone,
  type QueueStatus,
  type QueueStatusKind,
  type StatusTone,
} from "@/lib/queue-status";
import { cn } from "@/lib/utils";

/** Glyph of each status kind (design 8.1). */
const GLYPHS: Readonly<Record<QueueStatusKind, LucideIcon>> = {
  queued: Clock,
  probing: Loader,
  analyzing: Loader,
  ready: CircleDot,
  exporting: Loader,
  waiting: Clock,
  verifying: ScanSearch,
  done: CircleCheck,
  warnings: TriangleAlert,
  unverified: ShieldQuestionMark,
  failed: CircleX,
  canceled: CircleSlash,
};

/** Kinds whose glyph spins while the job runs. */
const SPINNING: ReadonlySet<QueueStatusKind> = new Set(["probing", "analyzing", "exporting"]);

/** Glyph colour of each tone; the status text next to it says the same. */
const TONES: Readonly<Record<StatusTone, string>> = {
  neutral: "text-foreground",
  muted: "text-foreground-subtlest",
  success: "text-success",
  warning: "text-warning",
  destructive: "text-destructive",
};

export interface StatusGlyphProps {
  readonly status: QueueStatus;
  readonly className?: string;
}

/** The status glyph of a queue item; decorative, the status text is announced. */
export function StatusGlyph({ status, className }: StatusGlyphProps) {
  const Glyph = GLYPHS[status.kind];
  return (
    <Glyph
      aria-hidden
      data-status={status.kind}
      className={cn(
        "size-3 shrink-0",
        TONES[statusTone(status)],
        SPINNING.has(status.kind) && "animate-spin motion-reduce:animate-none",
        className,
      )}
    />
  );
}
