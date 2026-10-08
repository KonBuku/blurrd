import { Kbd } from "@/components/ui/kbd";
import { SHORTCUTS } from "@/lib/shortcuts";

import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** Settings › Shortcuts: the keyboard shortcuts of the window. */
export function ShortcutSettings() {
  return (
    <SettingsPage title="Shortcuts" description="Keyboard shortcuts of the main window.">
      <SettingsGroup>
        {Object.values(SHORTCUTS).map((shortcut) => (
          <SettingsRow
            key={shortcut.id}
            label={shortcut.label}
            control={
              <span className="flex items-center gap-1">
                {shortcut.keys.map((key) => (
                  <Kbd key={key}>{key}</Kbd>
                ))}
              </span>
            }
          />
        ))}
      </SettingsGroup>
    </SettingsPage>
  );
}
