/**
 * The stages of a job (design 8.3 stepper). Each kind of job runs a fixed
 * part of the pipeline; the processing card shows those stages in
 * pipeline order and marks the ones the job has passed.
 */
import type { JobKind } from "@/bindings/JobKind";
import type { Stage } from "@/bindings/Stage";

/** Every stage in pipeline order (the order of the `Stage` binding). */
const PIPELINE: readonly Stage[] = [
  "Probe",
  "Decode",
  "Detect",
  "Track",
  "Ocr",
  "Verify",
  "BlurEncode",
  "Mux",
  "Audit",
];

/** The stages each kind of job reports, in pipeline order (plan 7). */
export const JOB_STAGES: Readonly<Record<JobKind, readonly Stage[]>> = {
  Analyze: ["Decode", "Detect", "Track", "Ocr", "Verify"],
  Render: ["BlurEncode", "Mux"],
  Run: ["Decode", "Detect", "Track", "Ocr", "Verify", "BlurEncode", "Mux"],
  Audit: ["Decode", "Audit"],
  ReverifyRanges: ["Decode", "Ocr", "Verify"],
};

/** The name of a stage as the stepper shows it. */
export const STAGE_LABELS: Readonly<Record<Stage, string>> = {
  Probe: "Probe",
  Decode: "Decode",
  Detect: "Detect",
  Track: "Track",
  Ocr: "OCR",
  Verify: "Verify",
  BlurEncode: "Blur + encode",
  Mux: "Mux",
  Audit: "Audit",
};

/**
 * The steps of an operation made of the jobs `kinds` (for example a
 * re-verification followed by a render): the stages of every job plus
 * any stage a job reported outside its plan, once each, in pipeline
 * order.
 */
export function stepsOf(kinds: readonly JobKind[], reported: readonly Stage[] = []): Stage[] {
  const wanted = new Set<Stage>([...kinds.flatMap((kind) => JOB_STAGES[kind]), ...reported]);
  return PIPELINE.filter((stage) => wanted.has(stage));
}

/** Where a step stands relative to the stage that runs now. */
export type StepState = "done" | "current" | "pending";

/**
 * The state of each step while `current` runs (`null` before the first
 * stage): steps before it are done, later ones pending.
 */
export function stepStates(steps: readonly Stage[], current: Stage | null): StepState[] {
  const at = current === null ? -1 : steps.indexOf(current);
  return steps.map((_, index) => (index < at ? "done" : index === at ? "current" : "pending"));
}
