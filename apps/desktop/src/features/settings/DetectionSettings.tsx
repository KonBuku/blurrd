import { useId } from "react";

import type { BlurStyle } from "@/bindings/BlurStyle";
import type { Strength } from "@/bindings/Strength";
import { RegionIcon } from "@/components/ui/region-chip";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { REGION_DESCRIPTIONS } from "@/features/inspector/RegionsSection";
import { ALL_REGIONS, orderedRegions, type DetectedRegion } from "@/lib/job-settings";
import { REGIONS } from "@/lib/regions";
import { useJobSettingsStore } from "@/store/job-settings";

import { ReadOnlyNotice, SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** Blur methods by select value; the solid fill starts with the box average. */
const STYLES: Readonly<Record<string, { label: string; style: BlurStyle }>> = {
  PixelBlur: { label: "Pixel + blur", style: "PixelBlur" },
  Gaussian: { label: "Gaussian blur", style: "Gaussian" },
  Pixelate: { label: "Pixelate", style: "Pixelate" },
  Solid: { label: "Solid fill", style: { Solid: { color: "BoxMean" } } },
};

/** Named strengths (`Strength::block_px`: 8, 10, 14 reference pixels). */
const STRENGTHS: Readonly<Record<Exclude<Strength, { Custom: number }>, string>> = {
  Low: "Low",
  Medium: "Medium (default)",
  High: "High",
};

const HUD = ALL_REGIONS.filter((region) => REGIONS[region].group === "hud");
const SCREENS = ALL_REGIONS.filter((region) => REGIONS[region].group === "screens");

/**
 * Settings › Detection (plan 6.3 `defaults.regions` and `defaults.blur`):
 * which regions new files blur and how the blur looks. The box fit of
 * new files follows the preset (or the inspector per file).
 */
export function DetectionSettings() {
  const regions = useJobSettingsStore((state) => state.defaults.values.regions);
  const blur = useJobSettingsStore((state) => state.defaults.values.blur);
  const updateDefaults = useJobSettingsStore((state) => state.updateDefaults);
  const enabled = new Set(regions.enabled);
  const styleLabel = useId();
  const strengthLabel = useId();
  const styleValue = typeof blur.style === "string" ? blur.style : "Solid";
  const strengthValue = typeof blur.strength === "string" ? blur.strength : "Custom";

  const setRegion = (region: DetectedRegion, on: boolean) =>
    updateDefaults("regions", (current) => {
      const next = new Set(current.enabled);
      if (on) next.add(region);
      else next.delete(region);
      return { ...current, enabled: orderedRegions(next) };
    });

  const row = (region: DetectedRegion) => (
    <RegionRow key={region} region={region} on={enabled.has(region)} onToggle={setRegion} />
  );

  return (
    <SettingsPage
      title="Detection"
      description="What new recordings blur and how. The inspector still changes these per file."
    >
      <ReadOnlyNotice />
      <section aria-label="HUD regions" className="flex flex-col gap-2">
        <h2 className="px-1 text-ui-base font-medium">HUD</h2>
        <SettingsGroup>{HUD.map(row)}</SettingsGroup>
      </section>
      <section aria-label="Screen regions" className="flex flex-col gap-2">
        <h2 className="px-1 text-ui-base font-medium">Screens</h2>
        <SettingsGroup>{SCREENS.map(row)}</SettingsGroup>
        <p className="px-1 text-ui-base text-foreground-subtle">
          Name tags above players&apos; heads in the game world are out of scope and are never
          blurred.
        </p>
      </section>
      <section aria-label="Blur" className="flex flex-col gap-2">
        <h2 className="px-1 text-ui-base font-medium">Blur</h2>
        <SettingsGroup>
          <SettingsRow
            label="Method"
            labelId={styleLabel}
            control={
              <Select
                value={styleValue}
                onValueChange={(value) =>
                  updateDefaults("blur", (current) => ({
                    ...current,
                    style: STYLES[value]?.style ?? current.style,
                  }))
                }
              >
                <SelectTrigger size="lg" aria-labelledby={styleLabel} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {Object.entries(STYLES).map(([value, { label }]) => (
                    <SelectItem key={value} value={value}>
                      {label}
                    </SelectItem>
                  ))}
                </SelectContent>
              </Select>
            }
          />
          <SettingsRow
            label="Strength"
            labelId={strengthLabel}
            description="Block size of the blur; higher hides more of the letters' shapes."
            control={
              <Select
                value={strengthValue}
                onValueChange={(value) =>
                  updateDefaults("blur", (current) => ({
                    ...current,
                    strength: value === "Custom" ? current.strength : (value as Strength),
                  }))
                }
              >
                <SelectTrigger size="lg" aria-labelledby={strengthLabel} className="w-full">
                  <SelectValue />
                </SelectTrigger>
                <SelectContent>
                  {(Object.keys(STRENGTHS) as (keyof typeof STRENGTHS)[]).map((value) => (
                    <SelectItem key={value} value={value}>
                      {STRENGTHS[value]}
                    </SelectItem>
                  ))}
                  {typeof blur.strength !== "string" && (
                    <SelectItem value="Custom">{`Custom (${blur.strength.Custom} px)`}</SelectItem>
                  )}
                </SelectContent>
              </Select>
            }
          />
        </SettingsGroup>
      </section>
    </SettingsPage>
  );
}

interface RegionRowProps {
  readonly region: DetectedRegion;
  readonly on: boolean;
  readonly onToggle: (region: DetectedRegion, on: boolean) => void;
}

/** One region: its icon, name, what it blurs and its switch. */
function RegionRow({ region, on, onToggle }: RegionRowProps) {
  const labelId = useId();
  return (
    <SettingsRow
      label={
        <span className="flex items-center gap-2">
          <RegionIcon region={region} />
          {REGIONS[region].label}
        </span>
      }
      labelId={labelId}
      description={REGION_DESCRIPTIONS[region]}
      control={
        <Switch
          aria-labelledby={labelId}
          checked={on}
          onCheckedChange={(checked) => onToggle(region, checked)}
        />
      }
    />
  );
}
