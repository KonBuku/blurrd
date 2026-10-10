import { useId } from "react";

import type { BlurParams } from "@/bindings/BlurParams";
import type { BlurStyle } from "@/bindings/BlurStyle";
import type { Strength } from "@/bindings/Strength";
import { NumberField } from "@/components/ui/number-field";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup } from "@/components/ui/toggle-group";
import { DEFAULT_BLUR } from "@/lib/job-settings";

import { InspectorRow, InspectorSection, SliderRow } from "./parts";

/** Kinds of blur style, as the method select names them. */
type StyleKind = "PixelBlur" | "Gaussian" | "Pixelate" | "Solid";

const STYLE_LABELS: Readonly<Record<StyleKind, string>> = {
  PixelBlur: "Pixel + blur",
  Gaussian: "Gaussian blur",
  Pixelate: "Pixelate",
  Solid: "Solid fill",
};

/** Fill colours of the solid style: the box average, black or a chosen colour. */
type FillKind = "BoxMean" | "Black" | "Custom";

/** Named strengths and a custom block size. */
type StrengthKind = "Low" | "Medium" | "High" | "Custom";

/** Block sizes of the named strengths (`Strength::block_px`), in reference pixels. */
const STRENGTH_PX = { Low: 8, Medium: 10, High: 14 } as const;
/** Smallest and largest custom block size (`MIN_BLOCK_PX` and a sane upper limit). */
const CUSTOM_PX = { min: 8, max: 64 } as const;
/** Largest padding, corner radius and feather offered, in reference pixels. */
const MAX_PAD = 16;
const MAX_RADIUS = 12;
const MAX_FEATHER = 4;

function styleKind(style: BlurStyle): StyleKind {
  return typeof style === "string" ? style : "Solid";
}

function strengthKind(strength: Strength): StrengthKind {
  return typeof strength === "string" ? strength : "Custom";
}

function blockPx(strength: Strength): number {
  return typeof strength === "string" ? STRENGTH_PX[strength] : strength.Custom;
}

/** `#rrggbb` of an RGB triple. */
function hex([r, g, b]: readonly [number, number, number]): string {
  return `#${[r, g, b].map((v) => v.toString(16).padStart(2, "0")).join("")}`;
}

/** The RGB triple of `#rrggbb` (the colour input always gives this form). */
function rgb(value: string): [number, number, number] {
  const n = Number.parseInt(value.slice(1), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255];
}

export interface StyleSectionProps {
  readonly blur: BlurParams;
  readonly onChange: (change: (blur: BlurParams) => BlurParams) => void;
}

/**
 * Inspector › Blur › Style: method, solid fill colour, strength (8 / 10 /
 * 14 px blocks or a custom size), padding, corners, feather and noise.
 * Sizes are in reference pixels of a 1080p HUD and scale with the video.
 */
export function StyleSection({ blur, onChange }: StyleSectionProps) {
  const methodId = useId();
  const fillId = useId();
  const strengthId = useId();
  const padId = useId();
  const noiseId = useId();
  const noiseDescriptionId = useId();
  const kind = styleKind(blur.style);
  const fill: FillKind | null =
    typeof blur.style === "string"
      ? null
      : blur.style.Solid.color === "BoxMean"
        ? "BoxMean"
        : blur.style.Solid.color.Rgb.every((v) => v === 0)
          ? "Black"
          : "Custom";
  const solidRgb =
    typeof blur.style !== "string" && blur.style.Solid.color !== "BoxMean"
      ? blur.style.Solid.color.Rgb
      : null;

  const set = <K extends keyof BlurParams>(key: K, value: BlurParams[K]) =>
    onChange((current) => ({ ...current, [key]: value }));

  return (
    <InspectorSection title="Style">
      <InspectorRow
        label="Method"
        labelId={methodId}
        control={
          <Select
            value={kind}
            onValueChange={(next) => {
              const style: BlurStyle =
                next === "Solid"
                  ? { Solid: { color: "BoxMean" } }
                  : (next as Exclude<StyleKind, "Solid">);
              set("style", style);
            }}
          >
            <SelectTrigger size="sm" aria-labelledby={methodId} className="w-36">
              <SelectValue />
            </SelectTrigger>
            <SelectContent>
              {(Object.keys(STYLE_LABELS) as StyleKind[]).map((value) => (
                <SelectItem key={value} value={value}>
                  {STYLE_LABELS[value]}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        }
      />
      {fill !== null && (
        <InspectorRow label="Fill colour" labelId={fillId}>
          <div className="flex items-center gap-2">
            <ToggleGroup<FillKind>
              size="sm"
              aria-label="Fill colour"
              value={fill}
              options={[
                { value: "BoxMean", label: "Box average" },
                { value: "Black", label: "Black" },
                { value: "Custom", label: "Custom" },
              ]}
              onValueChange={(next) => {
                const color =
                  next === "BoxMean"
                    ? ("BoxMean" as const)
                    : {
                        Rgb:
                          next === "Black"
                            ? ([0, 0, 0] as [number, number, number])
                            : ([32, 32, 32] as [number, number, number]),
                      };
                set("style", { Solid: { color } });
              }}
            />
            {fill === "Custom" && solidRgb && (
              <input
                type="color"
                aria-label="Custom fill colour"
                value={hex(solidRgb)}
                onChange={(event) =>
                  set("style", { Solid: { color: { Rgb: rgb(event.target.value) } } })
                }
                className="h-6 w-8 cursor-default rounded-md border border-input-border bg-input p-0.5"
              />
            )}
          </div>
        </InspectorRow>
      )}
      <InspectorRow
        label="Strength"
        labelId={strengthId}
        description={`${blockPx(blur.strength)} px blocks at 1080p`}
      >
        <div className="flex flex-wrap items-center gap-2">
          <ToggleGroup<StrengthKind>
            size="sm"
            aria-label="Strength"
            value={strengthKind(blur.strength)}
            options={[
              { value: "Low", label: "Low" },
              { value: "Medium", label: "Medium" },
              { value: "High", label: "High" },
              { value: "Custom", label: "Custom" },
            ]}
            onValueChange={(next) =>
              set("strength", next === "Custom" ? { Custom: blockPx(blur.strength) } : next)
            }
          />
          {typeof blur.strength !== "string" && (
            <NumberField
              aria-label="Custom block size"
              className="w-20"
              unit="px"
              min={CUSTOM_PX.min}
              max={CUSTOM_PX.max}
              value={blur.strength.Custom}
              onValueChange={(px) => set("strength", { Custom: px })}
            />
          )}
        </div>
      </InspectorRow>
      <InspectorRow
        label="Padding"
        labelId={padId}
        control={
          <fieldset aria-labelledby={padId} className="flex items-center gap-1.5">
            <NumberField
              aria-label="Horizontal padding"
              className="w-16"
              unit="px"
              min={0}
              max={MAX_PAD}
              value={blur.pad[0]}
              onValueChange={(x) =>
                onChange((current) => ({ ...current, pad: [x, current.pad[1]] }))
              }
            />
            <NumberField
              aria-label="Vertical padding"
              className="w-16"
              unit="px"
              min={0}
              max={MAX_PAD}
              value={blur.pad[1]}
              onValueChange={(y) =>
                onChange((current) => ({ ...current, pad: [current.pad[0], y] }))
              }
            />
          </fieldset>
        }
      />
      <SliderRow
        label="Corners"
        unit="px"
        min={0}
        max={MAX_RADIUS}
        value={blur.radius}
        resetValue={DEFAULT_BLUR.radius}
        onValueChange={(radius) => set("radius", radius)}
      />
      <SliderRow
        label="Feather"
        unit="px"
        min={0}
        max={MAX_FEATHER}
        step={0.5}
        value={blur.feather}
        resetValue={DEFAULT_BLUR.feather}
        onValueChange={(feather) => set("feather", feather)}
      />
      <InspectorRow
        label="Noise"
        labelId={noiseId}
        description="Adds fine noise inside each box, so blurred names cannot be recovered by averaging frames."
        descriptionId={noiseDescriptionId}
        control={
          <Switch
            aria-labelledby={noiseId}
            aria-describedby={noiseDescriptionId}
            checked={blur.noise}
            onCheckedChange={(noise) => set("noise", noise)}
          />
        }
      />
    </InspectorSection>
  );
}
