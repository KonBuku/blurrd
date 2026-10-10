import { Pause } from "lucide-react";

import { Progress } from "@/components/ui/progress";
import { formatCount } from "@/lib/media";
import { cn } from "@/lib/utils";

import { useJobsStore } from "./jobs";
import { jobsTaskbarState } from "./useTaskbarProgress";

/**
 * The queue's overall progress (design 8.1 queue footer) while a batch
 * runs: how many of its files are done, the overall percentage (the same
 * one the taskbar button shows) and a thin bar.
 */
export function BatchProgress({ className }: { readonly className?: string }) {
  const size = useJobsStore((state) => state.batch?.size ?? 0);
  const done = useJobsStore((state) => state.batch?.ended.length ?? 0);
  const paused = useJobsStore((state) => state.paused);
  const percent = useJobsStore((state) => jobsTaskbarState(state).progress);
  if (size === 0) return null;

  const label = `${formatCount(done)} of ${formatCount(size)} ${size === 1 ? "file" : "files"} done`;
  return (
    <div
      data-testid="batch-progress"
      className={cn("flex flex-col gap-1.5 rounded-lg px-2.5 py-2", className)}
    >
      <div className="flex items-center gap-1.5 text-ui-sm">
        {paused && <Pause aria-hidden className="size-3 text-foreground-subtle" />}
        <span className="min-w-0 flex-1 truncate">{paused ? `Paused · ${label}` : label}</span>
        <span className="font-mono text-foreground-subtle tabular-nums">{percent}%</span>
      </div>
      <Progress
        value={percent}
        aria-label="Queue progress"
        aria-valuetext={`${percent}%, ${label}`}
      />
    </div>
  );
}
