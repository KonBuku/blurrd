import { Loader } from "lucide-react";

import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { summarizeGpu, type GpuHealth, type GpuSummary } from "@/lib/gpu";
import { cn } from "@/lib/utils";

import { useCapabilities } from "./useCapabilities";

/** Dot colour per health; the title next to it says the same. */
const DOTS: Readonly<Record<GpuHealth, string>> = {
  gpu: "bg-success",
  "cpu-fallback": "bg-warning",
  "no-encoder": "bg-destructive",
};

/** What the card shows: probing, the summary, or why it is unknown. */
type CardState =
  | { readonly kind: "probing" }
  | { readonly kind: "ready"; readonly summary: GpuSummary }
  | { readonly kind: "error"; readonly message: string };

function useGpuState(): CardState {
  const { data, error } = useCapabilities();
  if (error) return { kind: "error", message: "The GPU could not be checked." };
  if (!data || data.probe_state === "Pending") return { kind: "probing" };
  return { kind: "ready", summary: summarizeGpu(data) };
}

function Dot({ health }: { readonly health: GpuHealth }) {
  return <span aria-hidden className={cn("size-2 shrink-0 rounded-full", DOTS[health])} />;
}

const PROBING = "Probing GPU…";

export interface GpuStatusCardProps {
  /** `compact` for the queue footer, `full` for the empty state. */
  readonly variant: "compact" | "full";
  readonly className?: string;
}

/**
 * The GPU status card (design 8.1 and 8.2): device, GPU or CPU path and
 * capability chips. It renders at once with "Probing GPU…" and fills in
 * when the background probe has finished, so it never delays the first
 * paint. Changes are announced politely.
 */
export function GpuStatusCard({ variant, className }: GpuStatusCardProps) {
  const state = useGpuState();
  return variant === "compact" ? (
    <CompactCard state={state} className={className} />
  ) : (
    <FullCard state={state} className={className} />
  );
}

function CompactCard({ state, className }: { state: CardState; className?: string | undefined }) {
  const details =
    state.kind === "ready" ? [state.summary.title, ...state.summary.chips].join(" · ") : "";
  return (
    // Not a landmark: the empty state's card is the "GPU status" region.
    <div
      aria-busy={state.kind === "probing"}
      aria-live="polite"
      className={cn("flex flex-col gap-0.5 rounded-lg px-2.5 py-2", className)}
    >
      {state.kind === "probing" && (
        <p className="flex items-center gap-2 text-ui-sm text-foreground-subtle">
          <Loader aria-hidden className="size-3 animate-spin motion-reduce:animate-none" />
          {PROBING}
        </p>
      )}
      {state.kind === "error" && (
        <p className="text-ui-sm text-foreground-subtle">{state.message}</p>
      )}
      {state.kind === "ready" && (
        <>
          <p className="flex min-w-0 items-center gap-2 text-ui-sm font-medium">
            <Dot health={state.summary.health} />
            <span className="truncate">{state.summary.device}</span>
          </p>
          <p className="truncate pl-4 text-ui-xs text-foreground-subtle" title={details}>
            {details}
          </p>
        </>
      )}
    </div>
  );
}

function FullCard({ state, className }: { state: CardState; className?: string | undefined }) {
  const title =
    state.kind === "ready" ? state.summary.title : state.kind === "probing" ? "GPU" : "GPU unknown";
  return (
    <Card
      aria-label="GPU status"
      aria-busy={state.kind === "probing"}
      aria-live="polite"
      className={cn("gap-2 py-3", className)}
    >
      {/* A plain row, not CardHeader: a <header> here would read as a page banner. */}
      <div className="flex items-center gap-2 px-3">
        {state.kind === "ready" && <Dot health={state.summary.health} />}
        <h2 className="text-ui-base font-medium">{title}</h2>
      </div>
      <CardContent className="flex flex-col gap-2 px-3">
        {state.kind === "probing" && (
          <p className="flex items-center gap-2 text-ui-sm text-foreground-subtle">
            <Loader aria-hidden className="size-3 animate-spin motion-reduce:animate-none" />
            {PROBING}
          </p>
        )}
        {state.kind === "error" && (
          <p className="text-ui-sm text-foreground-subtle">{state.message}</p>
        )}
        {state.kind === "ready" && (
          <>
            <p className="text-ui-sm">
              {state.summary.device}
              {state.summary.vram && (
                <span className="text-foreground-subtle"> · {state.summary.vram}</span>
              )}
            </p>
            {state.summary.chips.length > 0 && (
              <ul aria-label="Capabilities" className="flex flex-wrap gap-1">
                {state.summary.chips.map((chip) => (
                  <li key={chip}>
                    <Badge variant="outline">{chip}</Badge>
                  </li>
                ))}
              </ul>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}
