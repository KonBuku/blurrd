import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";

import type { AudioMode } from "@/bindings/AudioMode";
import type { ConflictPolicy } from "@/bindings/ConflictPolicy";
import type { Container } from "@/bindings/Container";
import type { MetadataMode } from "@/bindings/MetadataMode";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { templateProblem } from "@/lib/export-options";
import { logWarning } from "@/lib/log";

import { fieldSetter, type ExportChange, type ExportSectionProps } from "./export-section";
import { InspectorRow, InspectorSection } from "./parts";

/** AAC bit rates offered, in kbit/s. */
const AAC_RATES = [128, 192, 256, 320] as const;
/** Default AAC bit rate when switching to AAC. */
const DEFAULT_AAC = 192;

type AudioKind = "Copy" | "Aac" | "Remove";
type FolderKind = "NextToSource" | "Folder";

function audioKind(audio: AudioMode): AudioKind {
  return typeof audio === "string" ? audio : "Aac";
}

/** Inspector › Export › Audio: copy, AAC at a bit rate, or remove; subtitles. */
export function AudioSection({ output, change }: ExportSectionProps) {
  const set = fieldSetter(change);
  const audioId = useId();
  return (
    <InspectorSection title="Audio">
      <InspectorRow label="Audio" labelId={audioId}>
        <div className="flex flex-wrap items-center gap-2">
          <ToggleGroup<AudioKind>
            size="sm"
            aria-label="Audio"
            value={audioKind(output.audio)}
            options={[
              { value: "Copy", label: "Copy" },
              { value: "Aac", label: "AAC" },
              { value: "Remove", label: "Remove" },
            ]}
            onValueChange={(kind) => set("audio", kind === "Aac" ? { Aac: DEFAULT_AAC } : kind)}
          />
          {typeof output.audio !== "string" && (
            <Select
              value={String(output.audio.Aac)}
              onValueChange={(kbps) => set("audio", { Aac: Number(kbps) })}
            >
              <SelectTrigger size="sm" aria-label="AAC bit rate" className="w-28">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {AAC_RATES.map((kbps) => (
                  <SelectItem key={kbps} value={String(kbps)}>
                    {kbps} kbit/s
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          )}
        </div>
      </InspectorRow>
      <SwitchRow
        label="Copy subtitles"
        checked={output.copy_subtitles}
        onChange={(copy_subtitles) => set("copy_subtitles", copy_subtitles)}
      />
    </InspectorSection>
  );
}

/**
 * Inspector › Export › File: container, metadata, output folder, file name
 * template and what happens when the file exists.
 */
export function FileSection({ output, change }: ExportSectionProps) {
  const set = fieldSetter(change);
  const containerId = useId();
  const metadataId = useId();
  const folderId = useId();
  const conflictId = useId();
  return (
    <InspectorSection title="File">
      <InspectorRow
        label="Container"
        labelId={containerId}
        control={
          <Select
            value={output.container}
            onValueChange={(value) => set("container", value as Container)}
          >
            <SelectTrigger size="sm" aria-labelledby={containerId} className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="Source">Same as source</SelectItem>
              <SelectItem value="Mp4">MP4</SelectItem>
              <SelectItem value="Mkv">MKV</SelectItem>
              <SelectItem value="Mov">MOV</SelectItem>
            </SelectContent>
          </Select>
        }
      />
      <InspectorRow
        label="Metadata"
        labelId={metadataId}
        description={
          output.metadata === "Technical"
            ? "Keeps only colour, rotation and timecode; recorder titles, comments and account tags are removed."
            : "Keeps all container metadata of the source, including recorder tags."
        }
      >
        <ToggleGroup<MetadataMode>
          size="sm"
          aria-label="Metadata"
          value={output.metadata}
          options={[
            { value: "Technical", label: "Technical only" },
            { value: "All", label: "All" },
          ]}
          onValueChange={(metadata) => set("metadata", metadata)}
        />
      </InspectorRow>
      <InspectorRow label="Folder" labelId={folderId}>
        <ToggleGroup<FolderKind>
          size="sm"
          aria-label="Output folder"
          value={typeof output.output.folder === "string" ? "NextToSource" : "Folder"}
          options={[
            { value: "NextToSource", label: "Next to source" },
            { value: "Folder", label: "Choose folder" },
          ]}
          onValueChange={(kind) => {
            if (kind === "NextToSource") {
              change((current) => ({
                ...current,
                output: { ...current.output, folder: "NextToSource" },
              }));
            } else void chooseFolder(change);
          }}
        />
        {typeof output.output.folder !== "string" && (
          <div className="flex min-w-0 items-center gap-1.5">
            <span
              title={output.output.folder.Folder}
              className="min-w-0 flex-1 truncate font-mono text-ui-sm text-foreground-subtle"
            >
              {output.output.folder.Folder}
            </span>
            <Button variant="outline" size="sm" onClick={() => void chooseFolder(change)}>
              <FolderOpen strokeWidth={1.5} />
              Browse…
            </Button>
          </div>
        )}
      </InspectorRow>
      <TemplateRow
        template={output.output.template}
        onChange={(template) =>
          change((current) => ({ ...current, output: { ...current.output, template } }))
        }
      />
      <InspectorRow
        label="If the file exists"
        labelId={conflictId}
        control={
          <Select
            value={output.output.on_conflict}
            onValueChange={(value) =>
              change((current) => ({
                ...current,
                output: { ...current.output, on_conflict: value as ConflictPolicy },
              }))
            }
          >
            <SelectTrigger size="sm" aria-labelledby={conflictId} className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              <SelectItem value="AddNumber">Add a number</SelectItem>
              <SelectItem value="Overwrite">Overwrite</SelectItem>
              <SelectItem value="Skip">Skip the file</SelectItem>
            </SelectContent>
          </Select>
        }
      />
    </InspectorSection>
  );
}

/** Opens the folder dialog and makes the chosen folder the output folder. */
async function chooseFolder(change: ExportChange) {
  try {
    const folder = await open({ directory: true, title: "Choose the output folder" });
    if (typeof folder !== "string") return;
    change((current) => ({
      ...current,
      output: { ...current.output, folder: { Folder: folder } },
    }));
  } catch (reason) {
    logWarning("the folder dialog failed", reason);
    toast.error("The folder dialog could not be opened");
  }
}

interface SwitchRowProps {
  readonly label: string;
  readonly checked: boolean;
  readonly onChange: (checked: boolean) => void;
}

/** A row with a label and a switch. */
function SwitchRow({ label, checked, onChange }: SwitchRowProps) {
  const id = useId();
  return (
    <InspectorRow
      label={label}
      labelId={id}
      control={<Switch aria-labelledby={id} checked={checked} onCheckedChange={onChange} />}
    />
  );
}

interface TemplateRowProps {
  readonly template: string;
  readonly onChange: (template: string) => void;
}

/**
 * The file name template: `{name}` stands for the source's file name; the
 * text is checked as it is typed and saved when it is valid (on blur or
 * Enter). An invalid name is shown with its reason and never saved.
 */
function TemplateRow({ template, onChange }: TemplateRowProps) {
  const id = useId();
  const errorId = useId();
  const [draft, setDraft] = useState<string | null>(null);
  const text = draft ?? template;
  const problem = templateProblem(text);
  const commit = () => {
    if (draft === null) return;
    if (templateProblem(draft) === null && draft.trim() !== template) onChange(draft.trim());
    if (templateProblem(draft) === null) setDraft(null);
  };
  return (
    <InspectorRow
      label={<label htmlFor={id}>File name</label>}
      description={
        problem ? (
          <span id={errorId} className="text-destructive">
            {problem}
          </span>
        ) : (
          <>
            <code className="font-mono">{"{name}"}</code> is the source&apos;s file name.
          </>
        )
      }
    >
      <div className="flex items-center gap-1.5">
        <Input
          id={id}
          size="sm"
          spellCheck={false}
          autoComplete="off"
          aria-invalid={problem !== null}
          aria-describedby={problem ? errorId : undefined}
          value={text}
          onChange={(event) => setDraft(event.target.value)}
          onBlur={commit}
          onKeyDown={(event) => {
            if (event.key === "Enter") commit();
            if (event.key === "Escape" && draft !== null) {
              event.preventDefault();
              setDraft(null);
            }
          }}
          className="font-mono"
        />
        <Button
          variant="outline"
          size="sm"
          aria-label="Insert the source name"
          onClick={() => setDraft(`${text}{name}`)}
        >
          {"{name}"}
        </Button>
      </div>
    </InspectorRow>
  );
}
