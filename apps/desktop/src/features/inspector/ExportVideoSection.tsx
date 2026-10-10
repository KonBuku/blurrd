import { TriangleAlert } from "lucide-react";
import { useId } from "react";

import type { Capabilities } from "@/bindings/Capabilities";
import type { Codec } from "@/bindings/Codec";
import type { MediaInfo } from "@/bindings/MediaInfo";
import type { NvencSessions } from "@/bindings/NvencSessions";
import type { RateControl } from "@/bindings/RateControl";
import type { Toggle } from "@/bindings/Toggle";
import { NumberField } from "@/components/ui/number-field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ToggleGroup } from "@/components/ui/toggle-group";
import {
  CODEC_LABELS,
  DEFAULT_CQ,
  OUTPUT_CODECS,
  chosenEncoder,
  codecProblem,
  effectiveCodec,
  encoderLabel,
  encodersFor,
  formatMbps,
  tenBitProblem,
} from "@/lib/export-options";
import { DEFAULT_EXPORT } from "@/lib/job-settings";
import { formatFps } from "@/lib/media";

import { fieldSetter, type ExportSectionProps } from "./export-section";
import { InspectorRow, InspectorSection, SliderField, SliderRow } from "./parts";

/** Encoder presets: 1 (fastest) to 7 (best quality). */
const PRESETS = { min: 1, max: 7 } as const;
/** Constant quality range of the encoders. */
const CQ = { min: 0, max: 51 } as const;
/** Default VBR target and peak when switching to a bit rate, in kbit/s. */
const DEFAULT_VBR = { target_kbps: 30_000, max_kbps: 45_000 } as const;
/** Largest VBR rate offered, in kbit/s. */
const MAX_KBPS = 400_000;

type RateKind = "MatchSource" | "Cq" | "Vbr";

function rateKind(rate: RateControl): RateKind {
  if ("MatchSource" in rate) return "MatchSource";
  return "Cq" in rate ? "Cq" : "Vbr";
}

export interface VideoSectionProps extends ExportSectionProps {
  readonly caps: Capabilities | null;
  /** The source's video facts, once known. */
  readonly media: MediaInfo | null;
}

/**
 * Inspector › Export › Video: codec (codecs without a working encoder are
 * disabled and the reason is shown), encoder, rate control, preset,
 * 10-bit, NVENC sessions; resolution and frame rate match the source.
 */
export function VideoSection({ output, change, caps, media }: VideoSectionProps) {
  const set = fieldSetter(change);
  const sourceCodec = media?.video_codec ?? null;
  const codecId = useId();
  const encoderId = useId();
  const tenBitId = useId();
  const sessionsId = useId();

  const codecNotes = OUTPUT_CODECS.map((codec) => codecProblem(caps, codec)).filter(
    (problem, index, all): problem is string => problem !== null && all.indexOf(problem) === index,
  );
  const selectedProblem = codecProblem(caps, output.codec);
  const encoders = encodersFor(caps, effectiveCodec(output.codec, sourceCodec));
  const encoder = chosenEncoder(caps, output, sourceCodec);
  const tenBit = tenBitProblem(caps, output, sourceCodec);
  const rate = rateKind(output.rate);
  const codecForCq = output.codec === "Source" ? "H264" : output.codec;

  return (
    <InspectorSection title="Video">
      <InspectorRow
        label="Codec"
        labelId={codecId}
        description={
          codecNotes.length > 0 ? (
            <span className="flex flex-col gap-0.5">
              {codecNotes.map((note) => (
                <span key={note}>{note}</span>
              ))}
            </span>
          ) : undefined
        }
      >
        <ToggleGroup<Codec>
          size="sm"
          aria-label="Codec"
          value={output.codec}
          options={(["Source", ...OUTPUT_CODECS] as const).map((codec) => ({
            value: codec,
            label: codec === "Source" ? "Source" : CODEC_LABELS[codec],
            disabled: codecProblem(caps, codec) !== null && codec !== output.codec,
          }))}
          onValueChange={(codec) =>
            change((current) => ({
              ...current,
              codec,
              encoder: "Auto",
              rate:
                "Cq" in current.rate && codec !== "Source"
                  ? { Cq: DEFAULT_CQ[codec] }
                  : current.rate,
            }))
          }
        />
        {selectedProblem !== null && output.codec !== "Source" && (
          <p role="alert" className="flex items-start gap-1.5 text-ui-sm text-destructive">
            <TriangleAlert aria-hidden strokeWidth={1.5} className="mt-0.5 size-3.5 shrink-0" />
            Choose another codec; this one cannot be exported on this PC.
          </p>
        )}
      </InspectorRow>
      <InspectorRow
        label="Encoder"
        labelId={encoderId}
        description={
          encoder && !encoder.hardware ? (
            <span className="flex items-center gap-1">
              <TriangleAlert aria-hidden strokeWidth={1.5} className="size-3.5 text-warning" />
              Runs on the CPU, which is much slower.
            </span>
          ) : undefined
        }
      >
        <Select
          value={typeof output.encoder === "string" ? "Auto" : output.encoder.Named}
          onValueChange={(name) => set("encoder", name === "Auto" ? "Auto" : { Named: name })}
        >
          <SelectTrigger size="sm" aria-labelledby={encoderId} className="w-full">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="Auto">
              {encoders[0] ? `Automatic (${encoderLabel(encoders[0].name)})` : "Automatic"}
            </SelectItem>
            {encoders.map((candidate) => (
              <SelectItem key={candidate.name} value={candidate.name}>
                {encoderLabel(candidate.name)}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </InspectorRow>
      <InspectorRow
        label="Quality"
        description={
          rate === "MatchSource"
            ? "Peak bit rate as a multiple of the source's."
            : rate === "Cq"
              ? "Constant quality: lower numbers are better and larger."
              : "Target and peak bit rate."
        }
      >
        <ToggleGroup<RateKind>
          size="sm"
          aria-label="Rate control"
          value={rate}
          options={[
            { value: "MatchSource", label: "Match source" },
            { value: "Cq", label: "Quality" },
            { value: "Vbr", label: "Bit rate" },
          ]}
          onValueChange={(kind) =>
            set(
              "rate",
              kind === "MatchSource"
                ? DEFAULT_EXPORT.rate
                : kind === "Cq"
                  ? { Cq: DEFAULT_CQ[codecForCq] }
                  : { Vbr: DEFAULT_VBR },
            )
          }
        />
        <RateFields rate={output.rate} onChange={(next) => set("rate", next)} />
      </InspectorRow>
      <SliderRow
        label="Preset"
        valueText={`P${output.preset}`}
        min={PRESETS.min}
        max={PRESETS.max}
        value={output.preset}
        resetValue={DEFAULT_EXPORT.preset}
        onValueChange={(preset) => set("preset", preset)}
        caption={["Faster", "Better"]}
      />
      <InspectorRow
        label="10-bit output"
        labelId={tenBitId}
        description={tenBit ?? "Auto keeps the source's bit depth."}
      >
        <ToggleGroup<Toggle>
          size="sm"
          aria-label="10-bit output"
          value={output.ten_bit}
          options={[
            { value: "Auto", label: "Auto" },
            { value: "On", label: "On", disabled: tenBit !== null && output.ten_bit !== "On" },
            { value: "Off", label: "Off" },
          ]}
          onValueChange={(ten_bit) => set("ten_bit", ten_bit)}
        />
      </InspectorRow>
      <InspectorRow
        label="NVENC sessions"
        labelId={sessionsId}
        description="Two sessions split the video and encode both halves at once on GPUs that allow it."
      >
        <ToggleGroup<NvencSessions>
          size="sm"
          aria-label="NVENC sessions"
          value={output.nvenc_sessions}
          options={[
            { value: "Auto", label: "Auto" },
            { value: "One", label: "1" },
            { value: "Two", label: "2" },
          ]}
          onValueChange={(sessions) => set("nvenc_sessions", sessions)}
        />
      </InspectorRow>
      <InspectorRow
        label="Resolution and frame rate"
        description={
          media
            ? `${media.width}×${media.height} · ${formatFps(media.fps)} fps, same as the source`
            : "Always the same as the source"
        }
      />
    </InspectorSection>
  );
}

interface RateFieldsProps {
  readonly rate: RateControl;
  readonly onChange: (rate: RateControl) => void;
}

/** The numbers of the chosen rate control. */
function RateFields({ rate, onChange }: RateFieldsProps) {
  if ("MatchSource" in rate) {
    return (
      <NumberField
        aria-label="Peak bit rate factor"
        className="w-24"
        unit="×"
        min={1}
        max={4}
        step={0.1}
        value={rate.MatchSource.max_factor}
        onValueChange={(max_factor) => onChange({ MatchSource: { max_factor } })}
      />
    );
  }
  if ("Cq" in rate) {
    return (
      <SliderField
        header
        label="Constant quality"
        valueText={`CQ ${rate.Cq}`}
        value={rate.Cq}
        min={CQ.min}
        max={CQ.max}
        onValueChange={(cq) => onChange({ Cq: cq })}
        caption={["Better", "Smaller"]}
      />
    );
  }
  const { target_kbps, max_kbps } = rate.Vbr;
  return (
    <div className="flex items-center gap-1.5">
      <NumberField
        aria-label="Target bit rate"
        className="w-28"
        unit="Mbit/s"
        min={0.5}
        max={MAX_KBPS / 1000}
        step={0.5}
        value={Number(formatMbps(target_kbps))}
        onValueChange={(mbps) => {
          const target = Math.round(mbps * 1000);
          onChange({ Vbr: { target_kbps: target, max_kbps: Math.max(max_kbps, target) } });
        }}
      />
      <NumberField
        aria-label="Peak bit rate"
        className="w-28"
        unit="Mbit/s"
        min={Number(formatMbps(target_kbps))}
        max={MAX_KBPS / 1000}
        step={0.5}
        value={Number(formatMbps(max_kbps))}
        onValueChange={(mbps) =>
          onChange({
            Vbr: { target_kbps, max_kbps: Math.max(target_kbps, Math.round(mbps * 1000)) },
          })
        }
      />
    </div>
  );
}
