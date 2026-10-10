import { useEffect, type ReactNode } from "react";
import { create } from "zustand";

import type { ShortcutId } from "@/lib/shortcuts";

/** Groups of the command palette, in the order they are listed. */
export const COMMAND_GROUPS = [
  "File",
  "Export",
  "Preview",
  "Blur",
  "View",
  "Theme",
  "Help",
] as const;

/** A group of the command palette. */
export type CommandGroup = (typeof COMMAND_GROUPS)[number];

/** One action the command palette offers. */
export interface PaletteCommand {
  /** Stable id, unique across every source. */
  readonly id: string;
  readonly group: CommandGroup;
  /** What the command does, as shown to the user. */
  readonly label: string;
  /** Other words the search matches (synonyms, the region name). */
  readonly keywords?: readonly string[];
  readonly icon?: ReactNode;
  /** The shortcut that does the same, shown next to the label. */
  readonly shortcut?: ShortcutId | undefined;
  /** A short state shown instead of a shortcut ("Current"). */
  readonly hint?: string | undefined;
  /** Why the command cannot run right now; it stays listed but disabled. */
  readonly disabledReason?: string | undefined;
  readonly run: () => void;
}

interface CommandState {
  /** Whether the palette is open. */
  readonly open: boolean;
  /** Commands by the source (feature) that registered them. */
  readonly sources: Readonly<Record<string, readonly PaletteCommand[]>>;
  readonly setOpen: (open: boolean) => void;
  readonly toggle: () => void;
  /** Replaces the commands of `source`; an empty list removes it. */
  readonly register: (source: string, commands: readonly PaletteCommand[]) => void;
}

/**
 * The command palette's state: whether it is open and the commands each
 * mounted feature offers. Features register their commands while they
 * are on screen (the preview's only while a recording is shown), so the
 * palette never lists an action that has nothing to act on.
 */
export const useCommandStore = create<CommandState>()((set) => ({
  open: false,
  sources: {},
  setOpen: (open) => set({ open }),
  toggle: () => set((state) => ({ open: !state.open })),
  register: (source, commands) =>
    set((state) => {
      const others = Object.fromEntries(
        Object.entries(state.sources).filter(([key]) => key !== source),
      );
      return { sources: commands.length === 0 ? others : { ...others, [source]: commands } };
    }),
}));

/**
 * How well a command matches the search (cmdk filter, 0 = hidden): every
 * typed word must appear in the label, the group or a keyword. A label
 * that starts with the search ranks first, then labels that contain every
 * word, then matches through keywords. No fuzzy matching, so a search
 * never picks an unrelated command.
 */
export function commandScore(label: string, extra: readonly string[], search: string): number {
  const query = search.trim().toLowerCase();
  const words = query.split(/\s+/).filter(Boolean);
  if (words.length === 0) return 1;
  const name = label.toLowerCase();
  const text = `${name} ${extra.join(" ").toLowerCase()}`;
  if (!words.every((word) => text.includes(word))) return 0;
  if (name.startsWith(query)) return 1;
  return words.every((word) => name.includes(word)) ? 0.8 : 0.5;
}

/** Every registered command, in group order and then in registration order. */
export function paletteCommands(
  sources: Readonly<Record<string, readonly PaletteCommand[]>>,
): readonly PaletteCommand[] {
  const all = Object.values(sources).flat();
  return COMMAND_GROUPS.flatMap((group) => all.filter((command) => command.group === group));
}

/**
 * Offers `commands` in the palette while the calling component is
 * mounted. Pass a memoised list: every new list replaces the old one.
 */
export function useCommandSource(source: string, commands: readonly PaletteCommand[]): void {
  const register = useCommandStore((state) => state.register);
  useEffect(() => {
    register(source, commands);
  }, [register, source, commands]);
  useEffect(() => () => register(source, []), [register, source]);
}
