import { Fragment } from "react";

import { Kbd } from "@/components/ui/kbd";
import { SHORTCUTS, type Shortcut, type ShortcutScope } from "@/lib/shortcuts";

import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** The groups of the page, in order. */
const SCOPES: readonly { scope: ShortcutScope; title: string }[] = [
  { scope: "window", title: "Window" },
  { scope: "preview", title: "Preview" },
];

/** The keys of a shortcut as chips; alternatives separated by a slash. */
function ShortcutKeys({ shortcut }: { readonly shortcut: Shortcut }) {
  return (
    <span className="flex items-center gap-1">
      {[shortcut, ...shortcut.alternatives].map((binding, index) => (
        <Fragment key={binding.keys.join("+")}>
          {index > 0 && <span className="text-foreground-subtle">/</span>}
          {binding.keys.map((key) => (
            <Kbd key={key}>{key}</Kbd>
          ))}
        </Fragment>
      ))}
    </span>
  );
}

/** Settings › Shortcuts: the keyboard shortcuts of the window and the preview. */
export function ShortcutSettings() {
  const all = Object.values(SHORTCUTS);
  return (
    <SettingsPage title="Shortcuts" description="Keyboard shortcuts of the main window.">
      {SCOPES.map(({ scope, title }) => (
        <section key={scope} aria-labelledby={`shortcuts-${scope}`} className="flex flex-col gap-2">
          <h2 id={`shortcuts-${scope}`} className="text-ui-sm font-medium text-foreground-subtle">
            {title}
          </h2>
          <SettingsGroup>
            {all
              .filter((shortcut) => shortcut.scope === scope)
              .map((shortcut) => (
                <SettingsRow
                  key={shortcut.id}
                  label={shortcut.label}
                  control={<ShortcutKeys shortcut={shortcut} />}
                />
              ))}
          </SettingsGroup>
        </section>
      ))}
    </SettingsPage>
  );
}
