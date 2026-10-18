import { open } from "@tauri-apps/plugin-dialog";
import { FolderOpen, RotateCcw } from "lucide-react";
import { useId } from "react";
import { toast } from "sonner";

import type { AudioMode } from "@/bindings/AudioMode";
import type { Codec } from "@/bindings/Codec";
import type { ConflictPolicy } from "@/bindings/ConflictPolicy";
import type { Container } from "@/bindings/Container";
import type { ExportSettings } from "@/bindings/ExportSettings";
import type { MetadataMode } from "@/bindings/MetadataMode";
import type { RateControl } from "@/bindings/RateControl";
import { Button } from "@/components/ui/button";
import { NumberField } from "@/components/ui/number-field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { useCapabilities } from "@/features/gpu/useCapabilities";
import { CODEC_LABELS, DEFAULT_CQ, OUTPUT_CODECS, codecProblem } from "@/lib/export-options";
import { DEFAULT_EXPORT } from "@/lib/job-settings";
import { logWarning } from "@/lib/log";
import { useJobSettingsStore } from "@/store/job-settings";

import { TemplateRow, VbrRows } from "./OutputRows";
import { ReadOnlyNotice, SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** AAC bit rates offered, in kbit/s. */
const AAC_RATES = [128, 192, 256, 320] as const;
/** Constant quality range of the encoders. */
const CQ = { min: 0, max: 51 } as const;
/** Default VBR target and peak when switching to a bit rate, in kbit/s. */
const DEFAULT_VBR = { target_kbps: 30_000, max_kbps: 45_000 } as const;

const CONTAINERS: Readonly<Record<Container, string>> = {
  Source: "Same as source",
  Mp4: "MP4",
  Mkv: "MKV",
  Mov: "MOV",
};

type RateKind = "MatchSource" | "Cq" | "Vbr";

function rateKind(rate: RateControl): RateKind {
  if ("MatchSource" in rate) return "MatchSource";
  return "Cq" in rate ? "Cq" : "Vbr";
}

/** Select value of an audio mode: "Copy", "Remove" or "Aac:<kbit/s>". */
function audioValue(audio: AudioMode): string {
  return typeof audio === "string" ? audio : `Aac:${audio.Aac}`;
}

function audioMode(value: string): AudioMode {
  if (value === "Copy" || value === "Remove") return value;
  return { Aac: Number(value.slice("Aac:".length)) };
}

/** The extension an example output gets with `container`. */
function exampleExtension(container: Container): string {
  return container === "Source" ? "mp4" : container.toLowerCase();
}

/**
 * Settings › Output defaults (plan 6.3 `defaults.export`, 6.4): where new
 * files are exported to and how they are named and encoded. These are the
 * settings files added from now on start with; the inspector still
 * changes them per file. Every value is checked by the backend before it
 * is saved.
 */
export function OutputSettings() {
  const output = useJobSettingsStore((state) => state.defaults.values.export);
  const updateDefaults = useJobSettingsStore((state) => state.updateDefaults);
  const caps = useCapabilities().data ?? null;
  const change = (patch: (current: ExportSettings) => ExportSettings) =>
    updateDefaults("export", patch);
  const set = <K extends keyof ExportSettings>(key: K, value: ExportSettings[K]) =>
    change((current) => ({ ...current, [key]: value }));
  const folderLabel = useId();
  const folderValue = useId();
  const conflictLabel = useId();
  const containerLabel = useId();
  const codecLabel = useId();
  const rateLabel = useId();
  const audioLabel = useId();
  const metadataLabel = useId();
  const folder = output.output.folder;
  const rate = rateKind(output.rate);

  const chooseFolder = async () => {
    try {
      const chosen = await open({ directory: true, title: "Choose the export folder" });
      if (typeof chosen !== "string") return;
      change((current) => ({
        ...current,
        output: { ...current.output, folder: { Folder: chosen } },
      }));
    } catch (reason) {
      logWarning("the folder dialog failed", reason);
      toast.error("The folder dialog could not be opened");
    }
  };

  return (
    <SettingsPage
      title="Output defaults"
      description="How new recordings are exported. The inspector still changes these per file."
    >
      <ReadOnlyNotice />
      <SettingsGroup>
        <SettingsRow
          label="Export folder"
          labelId={folderLabel}
          controlWidth="wide"
          description={
            <span id={folderValue} className="flex flex-col gap-0.5">
              {typeof folder === "string" ? (
                <span>Next to each recording, in the recording&apos;s own folder.</span>
              ) : (
                <span
                  data-testid="default-export-folder"
                  title={folder.Folder}
                  className="block truncate font-mono text-foreground"
                >
                  {folder.Folder}
                </span>
              )}
            </span>
          }
          control={
            <>
              <Button
                variant="outline"
                size="lg"
                aria-describedby={`${folderLabel} ${folderValue}`}
                onClick={() => void chooseFolder()}
              >
                <FolderOpen aria-hidden />
                Change…
              </Button>
              <Button
                variant="ghost"
                size="lg"
                aria-label="Reset the export folder to next to each recording"
                disabled={typeof folder === "string"}
                onClick={() =>
                  change((current) => ({
                    ...current,
                    output: { ...current.output, folder: "NextToSource" },
                  }))
                }
              >
                <RotateCcw aria-hidden />
                Reset
              </Button>
            </>
          }
        />
        <TemplateRow
          template={output.output.template}
          extension={exampleExtension(output.container)}
          onChange={(template) =>
            change((current) => ({ ...current, output: { ...current.output, template } }))
          }
        />
        <SettingsRow
          label="If the file exists"
          labelId={conflictLabel}
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
              <SelectTrigger size="lg" aria-labelledby={conflictLabel} className="w-full">
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
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow
          label="Container"
          labelId={containerLabel}
          control={
            <Select
              value={output.container}
              onValueChange={(value) => set("container", value as Container)}
            >
              <SelectTrigger size="lg" aria-labelledby={containerLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(Object.keys(CONTAINERS) as Container[]).map((container) => (
                  <SelectItem key={container} value={container}>
                    {CONTAINERS[container]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          label="Video codec"
          labelId={codecLabel}
          description={
            output.codec === "Source"
              ? "Each file keeps its own codec; the encoder is chosen automatically."
              : (codecProblem(caps, output.codec) ?? "The encoder is chosen automatically.")
          }
          control={
            <Select
              value={output.codec}
              onValueChange={(value) => {
                const codec = value as Codec;
                change((current) => ({
                  ...current,
                  codec,
                  encoder: "Auto",
                  rate:
                    "Cq" in current.rate && codec !== "Source"
                      ? { Cq: DEFAULT_CQ[codec] }
                      : current.rate,
                }));
              }}
            >
              <SelectTrigger size="lg" aria-labelledby={codecLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {(["Source", ...OUTPUT_CODECS] as const).map((codec) => (
                  <SelectItem
                    key={codec}
                    value={codec}
                    disabled={codecProblem(caps, codec) !== null && codec !== output.codec}
                  >
                    {CODEC_LABELS[codec]}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          label="Quality"
          labelId={rateLabel}
          description={
            "MatchSource" in output.rate
              ? `About the bit rate of the source, at most ${output.rate.MatchSource.max_factor} times it.`
              : "Cq" in output.rate
                ? "Constant quality: lower numbers are better and larger."
                : "A target and a peak bit rate."
          }
          control={
            <Select
              value={rate}
              onValueChange={(kind) => {
                const codec = output.codec === "Source" ? "H264" : output.codec;
                set(
                  "rate",
                  kind === "MatchSource"
                    ? DEFAULT_EXPORT.rate
                    : kind === "Cq"
                      ? { Cq: DEFAULT_CQ[codec] }
                      : { Vbr: DEFAULT_VBR },
                );
              }}
            >
              <SelectTrigger size="lg" aria-labelledby={rateLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="MatchSource">Match the source</SelectItem>
                <SelectItem value="Cq">Constant quality</SelectItem>
                <SelectItem value="Vbr">Bit rate</SelectItem>
              </SelectContent>
            </Select>
          }
        />
        {"Cq" in output.rate && (
          <SettingsRow
            label="Constant quality"
            control={
              <NumberField
                size="lg"
                aria-label="Constant quality"
                value={output.rate.Cq}
                min={CQ.min}
                max={CQ.max}
                onValueChange={(cq) => set("rate", { Cq: cq })}
                className="w-24"
              />
            }
          />
        )}
        {"Vbr" in output.rate && (
          <VbrRows
            target={output.rate.Vbr.target_kbps}
            peak={output.rate.Vbr.max_kbps}
            onChange={(target_kbps, max_kbps) => set("rate", { Vbr: { target_kbps, max_kbps } })}
          />
        )}
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow
          label="Audio"
          labelId={audioLabel}
          control={
            <Select
              value={audioValue(output.audio)}
              onValueChange={(value) => set("audio", audioMode(value))}
            >
              <SelectTrigger size="lg" aria-labelledby={audioLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="Copy">Copy</SelectItem>
                {AAC_RATES.map((kbps) => (
                  <SelectItem key={kbps} value={`Aac:${kbps}`}>
                    AAC {kbps} kbit/s
                  </SelectItem>
                ))}
                <SelectItem value="Remove">Remove</SelectItem>
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          label="Metadata"
          labelId={metadataLabel}
          controlWidth="wide"
          description={
            output.metadata === "Technical"
              ? "Keeps only colour, rotation and timecode; recorder titles, comments and account tags are removed."
              : "Keeps all container metadata of the source, including recorder tags."
          }
          control={
            <ToggleGroup<MetadataMode>
              aria-label="Metadata"
              value={output.metadata}
              options={[
                { value: "Technical", label: "Technical only" },
                { value: "All", label: "All" },
              ]}
              onValueChange={(metadata) => set("metadata", metadata)}
            />
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
