/**
 * Keyboard shortcuts (design 8.8). Each shortcut has a stable id, the
 * keys shown in tooltips and menus, one or more key bindings and a scope:
 * window shortcuts work everywhere, preview shortcuts only while a
 * recording is shown and the focused control does not use the key itself.
 */

/** Ids of the shortcuts the app handles. */
export type ShortcutId =
  | "addFiles"
  | "addFolder"
  | "removeFromQueue"
  | "toggleQueue"
  | "toggleInspector"
  | "openSettings"
  | "closeOverlay"
  | "playPause"
  | "shuttleBack"
  | "shuttleForward"
  | "previousFrame"
  | "nextFrame"
  | "backSecond"
  | "forwardSecond"
  | "goToStart"
  | "goToEnd"
  | "showBefore"
  | "showAfter"
  | "showSplit"
  | "toggleBoxes"
  | "nextMark"
  | "previousMark"
  | "zoomIn"
  | "zoomOut"
  | "zoomFit"
  | "manualBox"
  | "setIn"
  | "deleteBox"
  | "nudgeBox"
  | "growBox"
  | "undo"
  | "redo"
  | "cancelEdit";

/** Where a shortcut applies. */
export type ShortcutScope = "window" | "preview";

/** One key combination. */
export interface KeyBinding {
  /** Keys as shown to the user, for example `["Ctrl", "B"]`. */
  readonly keys: readonly string[];
  /** The main key: a lowercase letter or a `KeyboardEvent.key` value. */
  readonly key: string;
  readonly ctrl: boolean;
  readonly alt: boolean;
  /** Whether Shift is held; `null` accepts both (keys that need Shift on some layouts). */
  readonly shift: boolean | null;
}

/** One keyboard shortcut: its first binding plus optional alternatives. */
export interface Shortcut extends KeyBinding {
  readonly id: ShortcutId;
  /** What the shortcut does, as shown to the user. */
  readonly label: string;
  readonly scope: ShortcutScope;
  /** Other bindings that do the same (Space / K). */
  readonly alternatives: readonly KeyBinding[];
  /** Whether holding the keys repeats the action (frame steps, zoom). */
  readonly repeat: boolean;
}

/** A binding without Ctrl or Alt. */
function plain(key: string, label: string, shift = false): KeyBinding {
  return { keys: shift ? ["Shift", label] : [label], key, ctrl: false, alt: false, shift };
}

/** A binding with Alt (and Shift when `shift`). */
function alt(key: string, label: string, shift = false): KeyBinding {
  return {
    keys: shift ? ["Alt", "Shift", label] : ["Alt", label],
    key,
    ctrl: false,
    alt: true,
    shift,
  };
}

/** A binding with Ctrl. */
function ctrl(key: string, keys: readonly string[], options: Partial<KeyBinding> = {}): KeyBinding {
  return { keys, key, ctrl: true, alt: false, shift: false, ...options };
}

interface ShortcutOptions {
  readonly scope?: ShortcutScope;
  readonly alternatives?: readonly KeyBinding[];
  readonly repeat?: boolean;
}

/** Builds a shortcut; window scope without repeat unless given. */
function shortcut(
  id: ShortcutId,
  label: string,
  binding: KeyBinding,
  options: ShortcutOptions = {},
): Shortcut {
  return {
    id,
    label,
    ...binding,
    scope: options.scope ?? "window",
    alternatives: options.alternatives ?? [],
    repeat: options.repeat ?? false,
  };
}

const PREVIEW: ShortcutOptions = { scope: "preview" };
const PREVIEW_REPEAT: ShortcutOptions = { scope: "preview", repeat: true };

/** The shortcuts by id, in the order they are listed. */
export const SHORTCUTS: Readonly<Record<ShortcutId, Shortcut>> = {
  addFiles: shortcut("addFiles", "Add recordings", ctrl("o", ["Ctrl", "O"])),
  addFolder: shortcut(
    "addFolder",
    "Add a folder of recordings",
    ctrl("o", ["Ctrl", "Shift", "O"], { shift: true }),
  ),
  removeFromQueue: shortcut(
    "removeFromQueue",
    "Remove the selected recordings (in the queue)",
    plain("Delete", "Delete"),
  ),
  toggleQueue: shortcut("toggleQueue", "Show or hide the queue", ctrl("b", ["Ctrl", "B"])),
  toggleInspector: shortcut(
    "toggleInspector",
    "Show or hide the inspector",
    ctrl("b", ["Ctrl", "Alt", "B"], { alt: true }),
  ),
  openSettings: shortcut("openSettings", "Open settings", ctrl(",", ["Ctrl", ","])),
  closeOverlay: shortcut(
    "closeOverlay",
    "Close settings, menus and dialogs",
    plain("Escape", "Esc"),
  ),
  playPause: shortcut("playPause", "Play or pause", plain(" ", "Space"), {
    ...PREVIEW,
    alternatives: [plain("k", "K")],
  }),
  shuttleBack: shortcut(
    "shuttleBack",
    "Play backwards (press again for faster)",
    plain("j", "J"),
    PREVIEW,
  ),
  shuttleForward: shortcut(
    "shuttleForward",
    "Play forwards (press again for faster)",
    plain("l", "L"),
    PREVIEW,
  ),
  previousFrame: shortcut(
    "previousFrame",
    "Previous frame",
    plain("ArrowLeft", "←"),
    PREVIEW_REPEAT,
  ),
  nextFrame: shortcut("nextFrame", "Next frame", plain("ArrowRight", "→"), PREVIEW_REPEAT),
  backSecond: shortcut(
    "backSecond",
    "Back one second",
    plain("ArrowLeft", "←", true),
    PREVIEW_REPEAT,
  ),
  forwardSecond: shortcut(
    "forwardSecond",
    "Forward one second",
    plain("ArrowRight", "→", true),
    PREVIEW_REPEAT,
  ),
  goToStart: shortcut("goToStart", "Go to the first frame", plain("Home", "Home"), PREVIEW),
  goToEnd: shortcut("goToEnd", "Go to the last frame", plain("End", "End"), PREVIEW),
  showBefore: shortcut(
    "showBefore",
    "Show the recording before blurring",
    plain("b", "B"),
    PREVIEW,
  ),
  showAfter: shortcut("showAfter", "Show the blurred result", plain("a", "A"), PREVIEW),
  showSplit: shortcut("showSplit", "Show before and after side by side", plain("s", "S"), PREVIEW),
  // One key, two jobs: O ends the manual box (the out point to I's in
  // point) while there is one to end, otherwise it toggles the overlay.
  toggleBoxes: shortcut(
    "toggleBoxes",
    "Show or hide the box overlay; while drawing or with a manual box selected, end the box on this frame",
    plain("o", "O"),
    PREVIEW,
  ),
  nextMark: shortcut("nextMark", "Next detection or leak", plain("n", "N"), PREVIEW_REPEAT),
  previousMark: shortcut(
    "previousMark",
    "Previous detection or leak",
    plain("n", "N", true),
    PREVIEW_REPEAT,
  ),
  zoomIn: shortcut("zoomIn", "Zoom the preview in", ctrl("=", ["Ctrl", "="]), {
    ...PREVIEW_REPEAT,
    alternatives: [ctrl("+", ["Ctrl", "+"], { shift: null })],
  }),
  zoomOut: shortcut("zoomOut", "Zoom the preview out", ctrl("-", ["Ctrl", "-"]), PREVIEW_REPEAT),
  zoomFit: shortcut("zoomFit", "Fit the preview to the stage", ctrl("0", ["Ctrl", "0"]), PREVIEW),
  manualBox: shortcut("manualBox", "Draw a manual box", plain("m", "M"), PREVIEW),
  setIn: shortcut(
    "setIn",
    "Start the manual box on this frame (while drawing or with a manual box selected)",
    plain("i", "I"),
    PREVIEW,
  ),
  deleteBox: shortcut(
    "deleteBox",
    "Delete the selected manual box or edit, or hide the selected detection",
    plain("Delete", "Delete"),
    { ...PREVIEW, alternatives: [plain("Backspace", "Backspace")] },
  ),
  nudgeBox: shortcut("nudgeBox", "Move the selected box by one pixel", alt("ArrowLeft", "←"), {
    ...PREVIEW_REPEAT,
    alternatives: [alt("ArrowRight", "→"), alt("ArrowUp", "↑"), alt("ArrowDown", "↓")],
  }),
  growBox: shortcut(
    "growBox",
    "Resize the selected box by one pixel (right and bottom edge)",
    alt("ArrowLeft", "←", true),
    {
      ...PREVIEW_REPEAT,
      alternatives: [
        alt("ArrowRight", "→", true),
        alt("ArrowUp", "↑", true),
        alt("ArrowDown", "↓", true),
      ],
    },
  ),
  undo: shortcut("undo", "Undo the last box edit", ctrl("z", ["Ctrl", "Z"]), PREVIEW_REPEAT),
  redo: shortcut(
    "redo",
    "Redo the last undone box edit",
    ctrl("z", ["Ctrl", "Shift", "Z"], { shift: true }),
    {
      ...PREVIEW_REPEAT,
      alternatives: [ctrl("y", ["Ctrl", "Y"])],
    },
  ),
  cancelEdit: shortcut(
    "cancelEdit",
    "Stop drawing, or clear the box selection",
    plain("Escape", "Esc"),
    PREVIEW,
  ),
};

/** The bindings of a shortcut, the first one first. */
function bindings(item: Shortcut): readonly KeyBinding[] {
  return [item, ...item.alternatives];
}

/** Keys of a shortcut as shown to the user: `Ctrl+O`, `Space / K`. */
export function shortcutText(item: Shortcut): string {
  return bindings(item)
    .map((binding) => binding.keys.join("+"))
    .join(" / ");
}

/** Key names of `aria-keyshortcuts` where they differ from the shown ones. */
const ARIA_NAMES: Readonly<Record<string, string>> = {
  Ctrl: "Control",
  Esc: "Escape",
  "←": "ArrowLeft",
  "→": "ArrowRight",
  "↑": "ArrowUp",
  "↓": "ArrowDown",
  "+": "Plus",
};

/** The `aria-keyshortcuts` value of a shortcut (`Control+O`, `Space K`). */
export function ariaKeys(item: Shortcut): string {
  return bindings(item)
    .map((binding) => binding.keys.map((key) => ARIA_NAMES[key] ?? key).join("+"))
    .join(" ");
}

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

function matches(binding: KeyBinding, key: string, event: KeyboardEvent): boolean {
  return (
    binding.key === key &&
    binding.ctrl === (event.ctrlKey || event.metaKey) &&
    binding.alt === event.altKey &&
    (binding.shift === null || binding.shift === event.shiftKey)
  );
}

/** Finds the shortcut of `scope` that `event` triggers, if any. */
export function matchShortcut(event: KeyboardEvent, scope: ShortcutScope): Shortcut | undefined {
  const key = mainKey(event);
  return Object.values(SHORTCUTS).find(
    (candidate) =>
      candidate.scope === scope &&
      (!event.repeat || candidate.repeat) &&
      bindings(candidate).some((binding) => matches(binding, key, event)),
  );
}

/** Roles whose elements use the arrow, Home/End and Page keys themselves. */
const NAVIGATING_ROLES = [
  "listbox",
  "radiogroup",
  "slider",
  "tablist",
  "menu",
  "menubar",
  "grid",
  "tree",
  "spinbutton",
  "scrollbar",
]
  .map((role) => `[role="${role}"]`)
  .join(",");

/** Elements that act on Space. */
const PRESSABLE = [
  "button",
  "a[href]",
  "summary",
  ...[
    "button",
    "checkbox",
    "switch",
    "radio",
    "menuitem",
    "menuitemradio",
    "menuitemcheckbox",
    "option",
    "tab",
  ].map((role) => `[role="${role}"]`),
].join(",");

const NAVIGATION_KEYS: ReadonlySet<string> = new Set([
  "ArrowLeft",
  "ArrowRight",
  "ArrowUp",
  "ArrowDown",
  "Home",
  "End",
  "PageUp",
  "PageDown",
]);

/**
 * Whether the element an event comes from uses its key itself, so that a
 * preview shortcut must leave it alone: dialogs and menus take every key,
 * text fields every key without Ctrl, composite widgets (lists, sliders,
 * radio groups) their navigation keys, buttons and options Space, the
 * queue list its Delete and Backspace.
 */
export function targetOwnsKey(event: KeyboardEvent): boolean {
  const { target } = event;
  if (!(target instanceof Element)) return false;
  if (target.closest('[role="dialog"],[role="alertdialog"],[role="menu"]')) return true;
  const editable =
    target.closest("input,textarea,select") !== null ||
    (target instanceof HTMLElement && target.isContentEditable);
  // Text fields keep plain keys and their own undo and redo.
  const textUndo = (event.ctrlKey || event.metaKey) && /^[yz]$/i.test(event.key);
  if (editable && (!(event.ctrlKey || event.metaKey) || textUndo)) return true;
  if (NAVIGATION_KEYS.has(event.key) && target.closest(NAVIGATING_ROLES)) return true;
  if ((event.key === "Delete" || event.key === "Backspace") && target.closest('[role="listbox"]'))
    return true;
  return event.key === " " && target.closest(PRESSABLE) !== null;
}
