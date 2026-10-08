/**
 * App-wide keyboard shortcuts (design 8.8). Each shortcut has a stable id,
 * the keys shown in tooltips and menus, and a matcher for key events.
 */

/** Ids of the shortcuts the app handles. */
export type ShortcutId = "toggleQueue" | "toggleInspector" | "openSettings" | "closeOverlay";

/** One keyboard shortcut. */
export interface Shortcut {
  readonly id: ShortcutId;
  /** What the shortcut does, as shown to the user. */
  readonly label: string;
  /** Keys as shown to the user, for example `["Ctrl", "B"]`. */
  readonly keys: readonly string[];
  /** The main key: a lowercase letter or a `KeyboardEvent.key` value. */
  readonly key: string;
  readonly ctrl: boolean;
  readonly alt: boolean;
  readonly shift: boolean;
}

/** The shortcuts by id, in the order they are listed. */
export const SHORTCUTS: Readonly<Record<ShortcutId, Shortcut>> = {
  toggleQueue: {
    id: "toggleQueue",
    label: "Show or hide the queue",
    keys: ["Ctrl", "B"],
    key: "b",
    ctrl: true,
    alt: false,
    shift: false,
  },
  toggleInspector: {
    id: "toggleInspector",
    label: "Show or hide the inspector",
    keys: ["Ctrl", "Alt", "B"],
    key: "b",
    ctrl: true,
    alt: true,
    shift: false,
  },
  openSettings: {
    id: "openSettings",
    label: "Open settings",
    keys: ["Ctrl", ","],
    key: ",",
    ctrl: true,
    alt: false,
    shift: false,
  },
  closeOverlay: {
    id: "closeOverlay",
    label: "Close settings, menus and dialogs",
    keys: ["Esc"],
    key: "Escape",
    ctrl: false,
    alt: false,
    shift: false,
  },
};

/**
 * The main key of an event: the typed letter, or the letter of the
 * physical key when a modifier makes `key` something else (Ctrl+Alt is
 * AltGr on many layouts), otherwise `key`.
 */
function mainKey(event: KeyboardEvent): string {
  if (/^[a-z]$/i.test(event.key)) return event.key.toLowerCase();
  const letter = /^Key([A-Z])$/.exec(event.code)?.[1];
  return letter ? letter.toLowerCase() : event.key;
}

/** Finds the shortcut that `event` triggers, if any. */
export function matchShortcut(event: KeyboardEvent): Shortcut | undefined {
  if (event.repeat) return undefined;
  const key = mainKey(event);
  return Object.values(SHORTCUTS).find(
    (candidate) =>
      candidate.key === key &&
      candidate.ctrl === (event.ctrlKey || event.metaKey) &&
      candidate.alt === event.altKey &&
      candidate.shift === event.shiftKey,
  );
}
