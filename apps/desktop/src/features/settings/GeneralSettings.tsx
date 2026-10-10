import { useId } from "react";

import { Switch } from "@/components/ui/switch";
import { useSettingsStore } from "@/store/settings";

import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** Settings › General: how blurrd behaves between runs (design 8.5). */
export function GeneralSettings() {
  const general = useSettingsStore((state) => state.general);
  const status = useSettingsStore((state) => state.status);
  const update = useSettingsStore((state) => state.updateGeneral);
  const restoreLabel = useId();
  const notifyLabel = useId();
  const folderLabel = useId();

  return (
    <SettingsPage title="General" description="How blurrd behaves between runs.">
      {status === "read-only" && (
        <p className="text-ui-base text-warning">
          The settings file cannot be written. Changes apply until blurrd closes.
        </p>
      )}
      <SettingsGroup>
        <SettingsRow
          label="Restore the queue"
          labelId={restoreLabel}
          description="Open the recordings of the last session again on start. When off, blurrd keeps no list of your files."
          control={
            <Switch
              aria-labelledby={restoreLabel}
              checked={general.restore_queue}
              onCheckedChange={(restore_queue) => update({ restore_queue })}
            />
          }
        />
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow
          label="Notify when processing ends"
          labelId={notifyLabel}
          description="Show a Windows notification when the last export or analysis finishes or fails while blurrd is in the background."
          control={
            <Switch
              aria-labelledby={notifyLabel}
              checked={general.notify}
              onCheckedChange={(notify) => update({ notify })}
            />
          }
        />
        <SettingsRow
          label="Open the folder after exporting"
          labelId={folderLabel}
          description="Show the exported file in File Explorer when the exports are done."
          control={
            <Switch
              aria-labelledby={folderLabel}
              checked={general.open_folder_after_export}
              onCheckedChange={(open_folder_after_export) => update({ open_folder_after_export })}
            />
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
