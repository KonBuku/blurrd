/**
 * Wording of verification reports (plan 6.6, 7.2): result, leak status,
 * where a leak was found, and which leaks count. Reports carry no OCR
 * text, only frames, places, boxes and confidences.
 */
import type { Leak } from "@/bindings/Leak";
import type { LeakStatus } from "@/bindings/LeakStatus";
import type { LeakZone } from "@/bindings/LeakZone";
import type { VerifyState } from "@/bindings/VerifyState";
import type { VerifySummary } from "@/bindings/VerifySummary";

/** Badge text and tone of a result. */
const STATE_LABELS: Readonly<
  Record<
    VerifyState,
    { readonly text: string; readonly tone: "success" | "warning" | "destructive" }
  >
> = {
  Clean: { text: "Verified clean", tone: "success" },
  LeaksRemain: { text: "Readable names remain", tone: "destructive" },
  Unverified: { text: "Not verified", tone: "warning" },
};

/**
 * Badge of a report's result. A clean result with accepted leaks says so:
 * the names were readable and the user chose to keep them.
 */
export function stateLabel(summary: VerifySummary): (typeof STATE_LABELS)[VerifyState] {
  if (summary.state === "Clean" && summary.accepted > 0) {
    return { text: "Accepted with leaks", tone: "warning" };
  }
  return STATE_LABELS[summary.state];
}

/** What happened to a leak. */
export const STATUS_LABELS: Readonly<Record<LeakStatus, string>> = {
  Open: "Open",
  Fixed: "Fixed",
  Accepted: "Accepted",
  Info: "Information",
};

/** Where a leak was found. */
export const ZONE_LABELS: Readonly<Record<LeakZone, string>> = {
  NameSlot: "Name slot",
  ProtectedZone: "Region",
  FullFrame: "Whole frame",
};

/** A leak with its index in the report (the overlay's `leak:<index>` key). */
export interface IndexedLeak {
  readonly index: number;
  readonly leak: Leak;
}

/** Leaks that count (in a region), in report order. */
export function countedLeaks(leaks: readonly Leak[]): IndexedLeak[] {
  return leaks
    .map((leak, index) => ({ index, leak }))
    .filter(({ leak }) => leak.kind !== "OutOfScope");
}

/** Names found outside every region (information only, D8). */
export function outOfScope(leaks: readonly Leak[]): IndexedLeak[] {
  return leaks
    .map((leak, index) => ({ index, leak }))
    .filter(({ leak }) => leak.kind === "OutOfScope");
}
