import { useEffect, useMemo, useRef, useState } from "react";

import {
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  CommandShortcut,
} from "@/components/ui/command";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Kbd } from "@/components/ui/kbd";
import { SHORTCUTS } from "@/lib/shortcuts";

import {
  COMMAND_GROUPS,
  commandScore,
  paletteCommands,
  useCommandStore,
  type PaletteCommand,
} from "./registry";
import { useAppCommands } from "./useAppCommands";

/** Id of the palette panel; Ctrl+K inside it closes it again. */
export const COMMAND_PALETTE_ID = "command-palette";

/** The right-hand side of an item: why it is disabled, its state or its shortcut. */
function ItemHint({ command }: { readonly command: PaletteCommand }) {
  const text = command.disabledReason ?? command.hint;
  if (text !== undefined) {
    return <CommandShortcut className="text-ui-xs text-foreground-subtle">{text}</CommandShortcut>;
  }
  if (command.shortcut === undefined) return null;
  return (
    <CommandShortcut aria-hidden>
      {SHORTCUTS[command.shortcut].keys.map((key) => (
        <Kbd key={key}>{key}</Kbd>
      ))}
    </CommandShortcut>
  );
}

/**
 * The command palette (design 8.7, Ctrl+K): a search field over every
 * action the mounted features offer, grouped (File, Export, Preview,
 * Blur, View, Theme, Help). Enter or a click closes the palette, returns
 * the focus to where it was and then runs the action, so dialogs it opens
 * get the focus; Escape only closes it.
 */
export function CommandPalette() {
  useAppCommands();
  const open = useCommandStore((state) => state.open);
  const setOpen = useCommandStore((state) => state.setOpen);
  const sources = useCommandStore((state) => state.sources);
  const commands = useMemo(() => paletteCommands(sources), [sources]);
  const [search, setSearch] = useState("");

  // The element focused before the palette opened, and the command chosen
  // in it: both are handled once the palette has closed.
  const returnFocus = useRef<HTMLElement | null>(null);
  const pending = useRef<PaletteCommand | null>(null);

  // Reopened before the closing animation ended (no close focus event):
  // the chosen command still runs.
  useEffect(() => {
    if (!open || pending.current === null) return;
    const command = pending.current;
    pending.current = null;
    command.run();
  }, [open]);

  const close = () => {
    setOpen(false);
    setSearch("");
  };
  const run = (command: PaletteCommand) => {
    pending.current = command;
    close();
  };

  return (
    <Dialog open={open} onOpenChange={(next) => (next ? setOpen(true) : close())}>
      <DialogContent
        id={COMMAND_PALETTE_ID}
        showClose={false}
        className="top-[12%] translate-y-0 gap-0 overflow-hidden p-0 sm:max-w-xl"
        onOpenAutoFocus={() => {
          const active = document.activeElement;
          returnFocus.current = active instanceof HTMLElement ? active : null;
        }}
        onCloseAutoFocus={(event) => {
          // The palette has no trigger: focus goes back to where it was,
          // then the chosen command runs (and may move it to a dialog).
          event.preventDefault();
          if (returnFocus.current?.isConnected) returnFocus.current.focus();
          returnFocus.current = null;
          const command = pending.current;
          pending.current = null;
          command?.run();
        }}
      >
        <DialogTitle className="sr-only">Command palette</DialogTitle>
        <DialogDescription className="sr-only">
          Type to search the commands, then press Enter to run the highlighted one.
        </DialogDescription>
        <Command
          loop
          label="Search commands"
          // Items carry the label first, then the group and the keywords.
          filter={(value, query, keywords = []) =>
            commandScore(keywords[0] ?? value, keywords.slice(1), query)
          }
        >
          <CommandInput value={search} onValueChange={setSearch} placeholder="Search commands…" />
          <CommandList>
            <CommandEmpty>No matching commands</CommandEmpty>
            {COMMAND_GROUPS.map((group) => {
              const items = commands.filter((command) => command.group === group);
              if (items.length === 0) return null;
              return (
                <CommandGroup key={group} heading={group}>
                  {items.map((command) => (
                    <CommandItem
                      key={command.id}
                      value={command.id}
                      keywords={[command.label, group, ...(command.keywords ?? [])]}
                      disabled={command.disabledReason !== undefined}
                      onSelect={() => run(command)}
                    >
                      {command.icon}
                      <span className="min-w-0 truncate">{command.label}</span>
                      <ItemHint command={command} />
                    </CommandItem>
                  ))}
                </CommandGroup>
              );
            })}
          </CommandList>
        </Command>
      </DialogContent>
    </Dialog>
  );
}
