/**
 * Errors of IPC commands. Commands reject with an `AppError` (plan 10);
 * anything else (a missing command, a transport failure) is wrapped into
 * an `Internal` one so callers handle a single shape.
 */
import type { AppError } from "@/bindings/AppError";
import type { ErrorKind } from "@/bindings/ErrorKind";

const KINDS: ReadonlySet<unknown> = new Set<ErrorKind>([
  "Io",
  "NotFound",
  "UnsupportedMedia",
  "Decode",
  "Encode",
  "EncoderUnavailable",
  "DriverTooOld",
  "OcrUnavailable",
  "ModelMissing",
  "InvalidProject",
  "ProjectMismatch",
  "StaleRevision",
  "Canceled",
  "Internal",
]);

/** Whether `value` has the shape of an `AppError`. */
export function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const raw = value as Record<string, unknown>;
  return (
    KINDS.has(raw.kind) &&
    typeof raw.message === "string" &&
    (raw.detail === null || typeof raw.detail === "string")
  );
}

/** The `AppError` a command rejected with, or an `Internal` error for anything else. */
export function toAppError(reason: unknown): AppError {
  if (isAppError(reason)) return reason;
  const detail = reason instanceof Error ? reason.message : String(reason);
  return { kind: "Internal", message: "Something went wrong inside blurrd.", detail };
}
