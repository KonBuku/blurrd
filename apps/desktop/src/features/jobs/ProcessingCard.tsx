import { Check, Circle, Pause, Play } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Progress } from "@/components/ui/progress";
import { Spinner } from "@/components/ui/spinner";
import { operationFraction, operationSteps, type OperationProgress } from "@/lib/job-progress";
import { STAGE_LABELS, stepStates, type StepState } from "@/lib/job-stages";
import { formatCount, formatDuration } from "@/lib/media";
import { cn } from "@/lib/utils";

import { cancelItem, setPaused, useJobsStore } from "./jobs";

export interface ProcessingCardProps {
  /** The queue item whose operation the card shows. */
  readonly itemId: string;
}

/** The card's title: what runs, for which file. */
function title(progress: OperationProgress, name: string, paused: boolean): string {
  if (progress.step < 0) return `Waiting to export ${name}`;
  const verb = progress.operation === "export" ? "Exporting" : "Analyzing";
  return paused ? `Paused · ${verb.toLowerCase()} ${name}` : `${verb} ${name}`;
}

/** The figures line: speed, frames and time left, as far as they are known. */
function figures(progress: OperationProgress): string {
  const parts: string[] = [];
  if (progress.fps !== null) parts.push(`${formatCount(Math.round(progress.fps))} fps`);
  if (progress.total > 0) {
    parts.push(`${formatCount(progress.frame)} / ${formatCount(progress.total)} frames`);
  }
  if (progress.etaMs !== null) parts.push(`${formatDuration(progress.etaMs / 1000)} left`);
  if (progress.warnings.length > 0) {
    const count = progress.warnings.length;
    parts.push(`${formatCount(count)} ${count === 1 ? "warning" : "warnings"}`);
  }
  return parts.join(" · ");
}

const STEP_TEXT: Readonly<Record<StepState, string>> = {
  done: "text-foreground-subtle",
  current: "font-medium text-foreground",
  pending: "text-foreground-subtle",
};

/** One step of the stepper: a glyph that says its state, then its name. */
function Step({ label, state, paused }: { label: string; state: StepState; paused: boolean }) {
  return (
    <li
      className={cn("flex items-center gap-1", STEP_TEXT[state])}
      aria-current={state === "current" ? "step" : undefined}
    >
      {state === "done" && <Check aria-hidden className="size-3 text-success" />}
      {state === "current" &&
        (paused ? (
          <Pause aria-hidden className="size-3" />
        ) : (
          <Spinner label={`${label} running`} className="size-3" />
        ))}
      {state === "pending" && <Circle aria-hidden className="size-3" />}
      <span>{label}</span>
      <span className="sr-only">
        {state === "done" ? "(done)" : state === "pending" ? "(next)" : ""}
      </span>
    </li>
  );
}

/**
 * The processing card above the transport (design 8.3) while the shown
 * file has an operation: title with Pause and Cancel, the stepper of its
 * stages, the progress bar with the overall percentage and a line with
 * the speed, frames and time left. A file waiting in the run queue shows
 * only the title and Cancel. Cancel works at any stage, also before the
 * backend has answered the job id.
 */
export function ProcessingCard({ itemId }: ProcessingCardProps) {
  const operation = useJobsStore((state) => state.running[itemId]);
  const paused = useJobsStore((state) => state.paused);
  if (!operation) return null;

  const { progress, name } = operation;
  const waiting = progress.step < 0;
  const steps = operationSteps(progress);
  const states = stepStates(steps, progress.stage);
  const fraction = operationFraction(progress);
  const percent = fraction === null ? null : Math.floor(fraction * 100);
  const heading = title(progress, name, paused);
  const line = figures(progress);

  return (
    <section
      aria-label="Processing"
      data-testid="processing-card"
      className="mx-2 mt-2 flex shrink-0 flex-col gap-2 rounded-xl border border-card-border bg-card px-3 py-2.5"
    >
      <div className="flex items-center gap-2">
        <h2 className="min-w-0 flex-1 truncate text-ui-base font-medium">{heading}</h2>
        {!waiting && (
          <Button variant="ghost" size="sm" onClick={() => void setPaused(!paused)}>
            {paused ? <Play aria-hidden /> : <Pause aria-hidden />}
            {paused ? "Resume" : "Pause"}
          </Button>
        )}
        <Button
          variant="outline"
          size="sm"
          className="border-destructive text-destructive"
          disabled={operation.cancelRequested}
          onClick={() => void cancelItem(itemId)}
        >
          {operation.cancelRequested ? "Canceling…" : "Cancel"}
        </Button>
      </div>
      {waiting ? (
        <p className="text-ui-sm text-foreground-subtle">
          Starts when the files before it are done.
        </p>
      ) : (
        <>
          <ol
            aria-label="Stages"
            className="flex flex-wrap items-center gap-x-2 gap-y-1 text-ui-sm"
          >
            {steps.map((stage, index) => (
              <Step
                key={stage}
                label={STAGE_LABELS[stage]}
                state={states[index] ?? "pending"}
                paused={paused}
              />
            ))}
          </ol>
          <div className="flex items-center gap-2">
            <Progress
              value={percent}
              aria-label="Progress"
              aria-valuetext={percent === null ? undefined : `${percent}%`}
              className="flex-1"
            />
            <span className="w-9 text-right font-mono text-ui-sm text-foreground-subtle tabular-nums">
              {percent === null ? "" : `${percent}%`}
            </span>
          </div>
          {line !== "" && (
            <p
              data-testid="processing-figures"
              // The warnings themselves, on hover.
              title={progress.warnings.length > 0 ? progress.warnings.join("\n") : undefined}
              className="truncate font-mono text-ui-sm text-foreground-subtle tabular-nums"
            >
              {line}
            </p>
          )}
        </>
      )}
    </section>
  );
}
