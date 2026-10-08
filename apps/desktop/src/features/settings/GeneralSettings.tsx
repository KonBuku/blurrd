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
    </SettingsPage>
  );
}
