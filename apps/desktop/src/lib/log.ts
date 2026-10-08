import { error, warn } from "@tauri-apps/plugin-log";

/** Formats an unknown thrown value for a log line. */
function describe(reason: unknown): string {
  if (reason instanceof Error) return `${reason.name}: ${reason.message}`;
  return String(reason);
}

/**
 * Writes a warning to the app log (through the log plugin) and to the
 * developer console. Never throws.
 */
export function logWarning(context: string, reason: unknown): void {
  const message = `${context}: ${describe(reason)}`;
  console.warn(message);
  warn(message).catch(() => {
    // The log itself is unavailable; the console line above is all we can do.
  });
}

/** Writes an error to the app log. Never throws. */
function forward(message: string): void {
  error(message).catch(() => {
    // The log itself is unavailable; the browser console still shows it.
  });
}

/**
 * Forwards uncaught errors and unhandled promise rejections of the page to
 * the app log, so that frontend failures end up next to the Rust records.
 */
export function forwardUncaughtErrors(target: Window = window): void {
  target.addEventListener("error", (event) => {
    forward(`uncaught error: ${event.message}`);
  });
  target.addEventListener("unhandledrejection", (event) => {
    forward(`unhandled rejection: ${describe(event.reason)}`);
  });
}
