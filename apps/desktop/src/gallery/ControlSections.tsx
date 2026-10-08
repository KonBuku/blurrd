import { useState } from "react";

import { BlurStyleIcon, PixelateIcon } from "@/components/icons/processing";
import { Checkbox } from "@/components/ui/checkbox";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { NumberField } from "@/components/ui/number-field";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectSeparator,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Slider } from "@/components/ui/slider";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup } from "@/components/ui/toggle-group";

import { Section, Specimen } from "./Section";

const STRENGTHS = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "custom", label: "Custom" },
] as const;

const STYLES = [
  { value: "blur", label: "Blur", icon: <BlurStyleIcon /> },
  { value: "pixelate", label: "Pixelate", icon: <PixelateIcon /> },
] as const;

type Strength = (typeof STRENGTHS)[number]["value"];
type Style = (typeof STYLES)[number]["value"];

/** Text fields, selects, toggles, sliders and number fields. */
export function ControlSections() {
  const [strength, setStrength] = useState<Strength>("medium");
  const [style, setStyle] = useState<Style>("blur");
  const [blur, setBlur] = useState([10]);
  const [padding, setPadding] = useState(4);
  const [feather, setFeather] = useState(0.25);

  return (
    <>
      <Section title="Text fields">
        <Specimen label="md">
          <Input aria-label="Output name" defaultValue="{name}_blurred" className="w-56" />
        </Specimen>
        <Specimen label="placeholder">
          <Input aria-label="Ignore words" placeholder="Words to ignore" className="w-56" />
        </Specimen>
        <Specimen label="invalid">
          <Input aria-label="Folder" aria-invalid defaultValue="C:\\missing" className="w-56" />
        </Specimen>
        <Specimen label="lg">
          <Input aria-label="Search settings" size="lg" placeholder="Search" className="w-56" />
        </Specimen>
        <Specimen label="disabled">
          <Input aria-label="Disabled field" disabled defaultValue="Read only" className="w-40" />
        </Specimen>
      </Section>

      <Section title="Select">
        <Select defaultValue="h264">
          <SelectTrigger aria-label="Codec" className="w-48">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectGroup>
              <SelectLabel>Codec</SelectLabel>
              <SelectItem value="h264">H.264</SelectItem>
              <SelectItem value="hevc">HEVC</SelectItem>
              <SelectSeparator />
              <SelectItem value="av1" disabled>
                AV1 (not supported by this GPU)
              </SelectItem>
            </SelectGroup>
          </SelectContent>
        </Select>
        <Select>
          <SelectTrigger aria-label="Encoder" size="lg" className="w-64">
            <SelectValue placeholder="Choose an encoder" />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="nvenc">NVENC</SelectItem>
            <SelectItem value="mf">Media Foundation</SelectItem>
          </SelectContent>
        </Select>
      </Section>

      <Section title="Switches and checkboxes">
        <Label>
          <Switch defaultChecked />
          Verify with OCR
        </Label>
        <Label>
          <Switch />
          Blur chat messages
        </Label>
        <Label>
          <Switch size="sm" defaultChecked />
          Small
        </Label>
        <Label>
          <Switch disabled />
          Disabled
        </Label>
        <Label>
          <Checkbox defaultChecked />
          Kill feed
        </Label>
        <Label>
          <Checkbox checked="indeterminate" />
          All regions
        </Label>
        <Label>
          <Checkbox />
          Text chat
        </Label>
      </Section>

      <Section title="Segmented controls">
        <ToggleGroup
          aria-label="Blur strength"
          options={STRENGTHS}
          value={strength}
          onValueChange={setStrength}
        />
        <ToggleGroup
          aria-label="Blur style"
          size="sm"
          options={STYLES}
          value={style}
          onValueChange={setStyle}
        />
      </Section>

      <Section title="Slider and number fields" className="items-end">
        <div className="flex w-72 flex-col gap-1">
          <div className="flex items-center justify-between">
            <span id="gallery-strength" className="text-ui-base font-medium">
              Strength
            </span>
            <span className="font-mono text-ui-sm text-foreground-subtle tabular-nums">
              {blur[0]} px
            </span>
          </div>
          <Slider
            aria-labelledby="gallery-strength"
            min={8}
            max={32}
            value={blur}
            onValueChange={setBlur}
            resetValue={[10]}
          />
        </div>
        <Specimen label="padding">
          <NumberField
            aria-label="Padding"
            className="w-24"
            value={padding}
            onValueChange={setPadding}
            min={0}
            max={32}
            unit="px"
          />
        </Specimen>
        <Specimen label="feather">
          <NumberField
            aria-label="Feather"
            className="w-24"
            value={feather}
            onValueChange={setFeather}
            min={0}
            max={1}
            step={0.05}
          />
        </Specimen>
        <Specimen label="disabled">
          <NumberField
            aria-label="Sessions"
            className="w-20"
            value={2}
            onValueChange={() => undefined}
            disabled
          />
        </Specimen>
      </Section>
    </>
  );
}
