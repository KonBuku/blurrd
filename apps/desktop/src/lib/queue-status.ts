/**
 * The status of a file in the queue (design 8.1, "Per-file status model")
 * and its text. The glyph for each tone and kind lives in
 * `features/queue/StatusGlyph.tsx`; status is never shown by colour alone.
 */
import { formatCount, formatDuration } from "./media";

/** Where a file is in its life cycle. */
export type QueueStatus =
  /** Waiting to be analyzed. */
  | { readonly kind: "queued" }
  /** The file is being read (probe). */
  | { readonly kind: "probing"; readonly folder: boolean }
  /** Analysis is running. */
  | {
      readonly kind: "analyzing";
      readonly progress: number | null;
      readonly fps: number | null;
    }
  /** Analysis finished; `boxes` is the number of blur boxes found. */
  | { readonly kind: "ready"; readonly boxes: number | null }
  /** Rendering is running. */
  | {
      readonly kind: "exporting";
      readonly progress: number | null;
      readonly fps: number | null;
      readonly etaMs: number | null;
    }
  /**
   * Queued for export in the backend's run queue, behind other files;
   * `prior` is the status it returns to when the export never starts.
   */
  | { readonly kind: "waiting"; readonly prior: RestingStatus }
  /** Verification is running. */
  | { readonly kind: "verifying"; readonly pass: number; readonly passes: number }
  /** Exported; `verified` is false when verification was switched off. */
  | { readonly kind: "done"; readonly verified: boolean }
  /** Exported, but verification reported possible leaks. */
  | { readonly kind: "warnings"; readonly leaks: number }
  /**
   * Exported as `_UNVERIFIED` (verification could not finish); waits for
   * the user to review and accept the report (plan 7.2).
   */
  | { readonly kind: "unverified" }
  /** Failed with a short reason; `detail` holds technical information. */
  | { readonly kind: "failed"; readonly reason: string; readonly detail: string | null }
  /** Canceled by the user. */
  | { readonly kind: "canceled" };

/** Kinds of {@link QueueStatus}. */
export type QueueStatusKind = QueueStatus["kind"];

/** Statuses that survive a restart (no job is running). */
export type RestingStatus = Exclude<
  QueueStatus,
  { kind: "probing" | "analyzing" | "exporting" | "waiting" | "verifying" }
>;

/** Colour role of a status glyph; the text always says the same. */
export type StatusTone = "neutral" | "muted" | "success" | "warning" | "destructive";

const JOINER = " · ";

function percent(progress: number | null): string | null {
  return progress === null ? null : `${Math.floor(Math.min(Math.max(progress, 0), 1) * 100)}%`;
}

function fps(value: number | null): string | null {
  return value === null ? null : `${formatCount(Math.round(value))} fps`;
}

function plural(count: number, one: string, many: string): string {
  return `${formatCount(count)} ${count === 1 ? one : many}`;
}

/** Joins the parts that are present with the status separator. */
function parts(...items: (string | null)[]): string {
  return items.filter((item): item is string => item !== null).join(JOINER);
}

/** The status line of a queue item, for example "Analyzing · 46% · 820 fps". */
export function statusText(status: QueueStatus): string {
  switch (status.kind) {
    case "queued":
      return "Queued";
    case "probing":
      return status.folder ? "Reading folder…" : "Reading file…";
    case "analyzing":
      return parts("Analyzing", percent(status.progress), fps(status.fps));
    case "ready":
      return parts("Ready", status.boxes === null ? null : plural(status.boxes, "box", "boxes"));
    case "exporting":
      return parts(
        "Exporting",
        percent(status.progress),
        fps(status.fps),
        status.etaMs === null ? null : `${formatDuration(status.etaMs / 1000)} left`,
      );
    case "waiting":
      return "Waiting to export";
    case "verifying":
      return `Verifying${JOINER}pass ${status.pass}/${status.passes}`;
    case "done":
      return status.verified ? "Done · verified clean" : "Done · not verified";
    case "warnings":
      return `Done${JOINER}${plural(status.leaks, "possible leak", "possible leaks")}`;
    case "unverified":
      return "Unverified · review the report";
    case "failed":
      return status.reason;
    case "canceled":
      return "Canceled";
  }
}

/** Colour role of the status glyph. */
export function statusTone(status: QueueStatus): StatusTone {
  switch (status.kind) {
    case "queued":
    case "waiting":
    case "canceled":
      return "muted";
    case "done":
      return status.verified ? "success" : "neutral";
    case "warnings":
    case "unverified":
      return "warning";
    case "failed":
      return "destructive";
    default:
      return "neutral";
  }
}

/** Whether a job (probe, analysis, export or verification) is running or waiting to run. */
export function isRunning(status: QueueStatus): boolean {
  return (
    status.kind === "probing" ||
    status.kind === "analyzing" ||
    status.kind === "exporting" ||
    status.kind === "waiting" ||
    status.kind === "verifying"
  );
}

/** Whether the file has reached an end state ("Clear finished" removes these). */
export function isFinished(status: QueueStatus): boolean {
  return (
    status.kind === "done" ||
    status.kind === "warnings" ||
    status.kind === "unverified" ||
    status.kind === "failed" ||
    status.kind === "canceled"
  );
}

/**
 * Progress from 0 to 1 for the inline bar, `null` for a running job of
 * unknown length, `undefined` when no bar is shown.
 */
export function statusProgress(status: QueueStatus): number | null | undefined {
  switch (status.kind) {
    case "probing":
      return null;
    case "analyzing":
    case "exporting":
      return status.progress;
    case "verifying":
      return status.passes > 0 ? (status.pass - 1) / status.passes : null;
    default:
      return undefined;
  }
}

/**
 * The status a file returns to when the app restarts while a job runs:
 * an interrupted analysis is queued again, an interrupted export or
 * verification leaves the analysed file ready.
 */
export function restingStatus(status: QueueStatus): RestingStatus {
  switch (status.kind) {
    case "probing":
    case "analyzing":
      return { kind: "queued" };
    case "exporting":
    case "verifying":
      return { kind: "ready", boxes: null };
    case "waiting":
      return status.prior;
    default:
      return status;
  }
}

/** A non-negative integer from untrusted JSON, or `null`. */
function savedCount(field: unknown): number | null {
  return typeof field === "number" && Number.isInteger(field) && field >= 0 ? field : null;
}

/**
 * Reads a saved status from untrusted JSON (the session file); anything
 * unknown or malformed reads as queued.
 */
export function parseRestingStatus(value: unknown): RestingStatus {
  const raw = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  switch (raw.kind) {
    case "ready":
      return { kind: "ready", boxes: savedCount(raw.boxes) };
    case "done":
      return { kind: "done", verified: raw.verified === true };
    case "warnings":
      return { kind: "warnings", leaks: savedCount(raw.leaks) ?? 0 };
    case "unverified":
      return { kind: "unverified" };
    case "failed":
      return {
        kind: "failed",
        reason: typeof raw.reason === "string" && raw.reason !== "" ? raw.reason : "Failed",
        detail: typeof raw.detail === "string" ? raw.detail : null,
      };
    case "canceled":
      return { kind: "canceled" };
    default:
      return { kind: "queued" };
  }
}

/** Order of statuses for "Sort by status": what needs attention first. */
const SORT_ORDER: Readonly<Record<QueueStatusKind, number>> = {
  failed: 0,
  unverified: 1,
  warnings: 2,
  exporting: 3,
  verifying: 4,
  analyzing: 5,
  waiting: 6,
  probing: 7,
  ready: 8,
  queued: 9,
  done: 10,
  canceled: 11,
};

/** Rank of a status when sorting by status (lower first). */
export function statusRank(status: QueueStatus): number {
  return SORT_ORDER[status.kind];
}
