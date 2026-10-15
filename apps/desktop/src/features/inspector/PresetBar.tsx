import { useQuery, useQueryClient } from "@tanstack/react-query";
import { open, save } from "@tauri-apps/plugin-dialog";
import { Ellipsis } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";

import type { Preset } from "@/bindings/Preset";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toAppError } from "@/ipc/errors";
import { deletePreset, exportPreset, importPreset, listPresets, savePreset } from "@/ipc/presets";
import { logWarning } from "@/lib/log";
import { differsFromPreset, presetValues } from "@/lib/presets";
import { fileSettingsOf, targetsOf, useJobSettingsStore } from "@/store/job-settings";
import { shownItemId, useQueueStore } from "@/store/queue";

/** File type of exported presets (plan 6.5). */
const PRESET_FILTER = { name: "blurrd preset", extensions: ["blurrd-preset.json", "json"] };

/** Applies a preset to the inspector's targets. */
function apply(preset: Preset) {
  const store = useJobSettingsStore.getState();
  store.applyPreset(targetsOf(useQueueStore.getState(), store.scope), preset);
}

/** Shows an IPC failure as a toast. */
function failed(title: string, reason: unknown) {
  toast.error(title, { description: toAppError(reason).message });
}

/**
 * The inspector's preset row (design 8.1 "Preset ▾ ⋯"): the preset of the
 * shown file (or of new files) with a "modified" mark when its values
 * differ, and a menu to reset, save, update, import, export and delete
 * presets. Choosing a preset applies it to the inspector's targets.
 */
export function PresetBar() {
  const queryClient = useQueryClient();
  const presets = useQuery({ queryKey: ["presets"], queryFn: listPresets }).data ?? [];
  const leadId = useQueueStore(shownItemId);
  const entry = useJobSettingsStore((state) => fileSettingsOf(state, leadId));
  const preset = presets.find((candidate) => candidate.id === entry.presetId) ?? null;
  const modified = preset !== null && differsFromPreset(entry.values, preset);
  const [dialog, setDialog] = useState<"save" | "export" | null>(null);
  const labelId = useId();

  const setList = (list: Preset[]) => queryClient.setQueryData(["presets"], list);

  const update = async () => {
    if (!preset) return;
    try {
      setList(await savePreset({ ...preset, ...presetValues(entry.values) }));
      toast.success(`Preset “${preset.name}” updated`);
    } catch (reason) {
      failed("The preset could not be saved", reason);
    }
  };

  const remove = async () => {
    if (!preset) return;
    try {
      setList(await deletePreset(preset.id));
      toast.success(`Preset “${preset.name}” deleted`);
    } catch (reason) {
      failed("The preset could not be deleted", reason);
    }
  };

  const importFile = async () => {
    try {
      const path = await open({ title: "Import presets", filters: [PRESET_FILTER] });
      if (typeof path !== "string") return;
      const added = await importPreset(path);
      await queryClient.invalidateQueries({ queryKey: ["presets"] });
      toast.success(
        added.length === 1
          ? `Preset “${added[0]?.name ?? ""}” imported`
          : `${added.length} presets imported`,
      );
    } catch (reason) {
      logWarning("importing a preset failed", reason);
      failed("The presets could not be imported", reason);
    }
  };

  return (
    <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
      <span id={labelId} className="text-ui-sm text-foreground-subtle">
        Preset
      </span>
      <Select
        value={preset?.id ?? ""}
        onValueChange={(id) => {
          const chosen = presets.find((candidate) => candidate.id === id);
          if (chosen) apply(chosen);
        }}
      >
        <SelectTrigger size="sm" aria-labelledby={labelId} className="min-w-0 flex-1">
          <SelectValue placeholder="Custom" />
        </SelectTrigger>
        <SelectContent>
          {presets.map((candidate) => (
            <SelectItem key={candidate.id} value={candidate.id}>
              {candidate.name}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
      {modified && (
        <span data-testid="preset-modified" className="text-ui-xs text-foreground-subtle">
          Modified
        </span>
      )}
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button variant="ghost" size="icon-sm" aria-label="Preset actions">
            <Ellipsis strokeWidth={1.5} />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          {preset && modified && (
            <DropdownMenuItem onSelect={() => apply(preset)}>
              Reset to {preset.name}
            </DropdownMenuItem>
          )}
          {preset && modified && !preset.builtin && (
            <DropdownMenuItem onSelect={() => void update()}>Update {preset.name}</DropdownMenuItem>
          )}
          <DropdownMenuItem onSelect={() => setDialog("save")}>Save as preset…</DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => void importFile()}>Import presets…</DropdownMenuItem>
          {preset && (
            <DropdownMenuItem onSelect={() => setDialog("export")}>
              Export {preset.name}…
            </DropdownMenuItem>
          )}
          {preset && !preset.builtin && (
            <>
              <DropdownMenuSeparator />
              <DropdownMenuItem variant="destructive" onSelect={() => void remove()}>
                Delete {preset.name}
              </DropdownMenuItem>
            </>
          )}
        </DropdownMenuContent>
      </DropdownMenu>
      <SavePresetDialog
        open={dialog === "save"}
        onOpenChange={(next) => setDialog(next ? "save" : null)}
        onSave={async (name) => {
          const saved: Preset = {
            id: `user-${crypto.randomUUID()}`,
            name,
            builtin: false,
            ...presetValues(entry.values),
          };
          setList(await savePreset(saved));
          apply(saved);
          toast.success(`Preset “${name}” saved`);
        }}
      />
      {preset && (
        <ExportPresetDialog
          open={dialog === "export"}
          onOpenChange={(next) => setDialog(next ? "export" : null)}
          preset={preset}
        />
      )}
    </div>
  );
}

interface SavePresetDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** Saves the shown values under `name`; rejects with the failure. */
  readonly onSave: (name: string) => Promise<void>;
}

/** Asks for a name and saves the shown values as a new preset. */
function SavePresetDialog({ open: shown, onOpenChange, onSave }: SavePresetDialogProps) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);
  const id = useId();
  const submit = async () => {
    if (name.trim() === "") return;
    setBusy(true);
    try {
      await onSave(name.trim());
      setName("");
      onOpenChange(false);
    } catch (reason) {
      failed("The preset could not be saved", reason);
    } finally {
      setBusy(false);
    }
  };
  return (
    <Dialog open={shown} onOpenChange={onOpenChange}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Save as preset</DialogTitle>
          <DialogDescription>
            Saves the regions, blur, export and verification settings shown in the inspector.
          </DialogDescription>
        </DialogHeader>
        <form
          className="flex flex-col gap-1.5"
          onSubmit={(event) => {
            event.preventDefault();
            void submit();
          }}
        >
          <label htmlFor={id} className="text-ui-sm font-medium">
            Name
          </label>
          <Input
            id={id}
            autoComplete="off"
            value={name}
            onChange={(event) => setName(event.target.value)}
          />
          <DialogFooter className="mt-3">
            <Button variant="outline" onClick={() => onOpenChange(false)}>
              Cancel
            </Button>
            <Button type="submit" disabled={busy || name.trim() === ""}>
              Save
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

interface ExportPresetDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly preset: Preset;
}

/**
 * Exports a preset to a file. The names kept readable stay out of the
 * file unless the box is ticked (plan 6.5).
 */
function ExportPresetDialog({ open: shown, onOpenChange, preset }: ExportPresetDialogProps) {
  const [keepNames, setKeepNames] = useState(false);
  const id = useId();
  const names = preset.regions.keep_names.names.length;
  const run = async () => {
    try {
      const path = await save({
        title: "Export preset",
        defaultPath: `${preset.name}.blurrd-preset.json`,
        filters: [PRESET_FILTER],
      });
      if (path === null) return;
      await exportPreset(preset.id, path, keepNames);
      toast.success(`Preset “${preset.name}” exported`);
      onOpenChange(false);
    } catch (reason) {
      logWarning("exporting a preset failed", reason);
      failed("The preset could not be exported", reason);
    }
  };
  return (
    <Dialog
      open={shown}
      onOpenChange={(next) => {
        if (!next) setKeepNames(false);
        onOpenChange(next);
      }}
    >
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Export {preset.name}</DialogTitle>
          <DialogDescription>
            Writes the preset to a file you can share or import on another PC.
          </DialogDescription>
        </DialogHeader>
        <div className="flex items-start gap-2">
          <Checkbox
            id={id}
            className="mt-0.5"
            checked={keepNames}
            onCheckedChange={(checked) => setKeepNames(checked === true)}
          />
          <label htmlFor={id} className="text-ui-sm">
            Include the names kept readable
            <span className="block text-foreground-subtle">
              {names === 0
                ? "This preset keeps no names readable."
                : `${names} ${names === 1 ? "name" : "names"}; left out unless ticked.`}
            </span>
          </label>
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Cancel
          </Button>
          <Button onClick={() => void run()}>Export…</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
