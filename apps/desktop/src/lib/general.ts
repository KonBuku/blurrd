import type { General } from "@/bindings/General";

/** General behaviour of a fresh install (matches the Rust `General::default`). */
export const DEFAULT_GENERAL: General = {
  restore_queue: true,
  confirm_remove_running: true,
  open_folder_after_export: false,
  notify: true,
};

/**
 * Reads the general section from untrusted JSON: every field that is
 * missing or not a boolean takes its default, like the Rust deserializer.
 */
export function parseGeneral(value: unknown): General {
  const raw = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  const flag = (key: keyof General) => {
    const field = raw[key];
    return typeof field === "boolean" ? field : DEFAULT_GENERAL[key];
  };
  return {
    restore_queue: flag("restore_queue"),
    confirm_remove_running: flag("confirm_remove_running"),
    open_folder_after_export: flag("open_folder_after_export"),
    notify: flag("notify"),
  };
}
