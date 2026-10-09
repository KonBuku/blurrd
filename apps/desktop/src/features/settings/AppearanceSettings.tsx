import { Monitor, Moon, Sun } from "lucide-react";
import { useId } from "react";

import type { ReduceMotion } from "@/bindings/ReduceMotion";
import type { Theme } from "@/bindings/Theme";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, type ToggleOption } from "@/components/ui/toggle-group";
import { DEFAULT_APPEARANCE, UI_FONT_SIZES } from "@/lib/appearance";
import { useSettingsStore } from "@/store/settings";

import { SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

/** Theme choices, also used by the title bar theme menu. */
export const THEME_OPTIONS: readonly ToggleOption<Theme>[] = [
  { value: "System", label: "System", icon: <Monitor aria-hidden /> },
  { value: "Light", label: "Light", icon: <Sun aria-hidden /> },
  { value: "Dark", label: "Dark", icon: <Moon aria-hidden /> },
];

const MOTION_OPTIONS: readonly ToggleOption<ReduceMotion>[] = [
  { value: "System", label: "System" },
  { value: "On", label: "On" },
  { value: "Off", label: "Off" },
];

/**
 * Settings › Appearance: theme, interface size, motion and the timeline
 * thumbnails (design 8.5).
 */
export function AppearanceSettings() {
  const appearance = useSettingsStore((state) => state.appearance);
  const status = useSettingsStore((state) => state.status);
  const update = useSettingsStore((state) => state.updateAppearance);
  const sizeLabel = useId();
  const thumbsLabel = useId();

  return (
    <SettingsPage title="Appearance" description="How blurrd looks on this PC.">
      {status === "read-only" && (
        <p className="text-ui-base text-warning">
          The settings file cannot be written. Changes apply until blurrd closes.
        </p>
      )}
      <SettingsGroup>
        <SettingsRow
          label="Theme"
          description="Follow the Windows setting, or always use light or dark."
          controlWidth="wide"
          control={
            <ToggleGroup
              aria-label="Theme"
              options={THEME_OPTIONS}
              value={appearance.theme}
              onValueChange={(theme) => update({ theme })}
            />
          }
        />
        <SettingsRow
          label="Interface size"
          labelId={sizeLabel}
          description="Text size of the whole interface."
          control={
            <Select
              value={String(appearance.ui_font_size)}
              onValueChange={(value) => update({ ui_font_size: Number(value) })}
            >
              <SelectTrigger size="lg" aria-labelledby={sizeLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {UI_FONT_SIZES.map((size) => (
                  <SelectItem key={size} value={String(size)}>
                    {size === DEFAULT_APPEARANCE.ui_font_size
                      ? `${size} px (default)`
                      : `${size} px`}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          label="Reduce motion"
          description="Turn off animations. System follows the Windows setting."
          controlWidth="wide"
          control={
            <ToggleGroup
              aria-label="Reduce motion"
              options={MOTION_OPTIONS}
              value={appearance.reduce_motion}
              onValueChange={(reduce_motion) => update({ reduce_motion })}
            />
          }
        />
        <SettingsRow
          label="Timeline thumbnails"
          labelId={thumbsLabel}
          description="Show a strip of frame thumbnails under the timeline lanes."
          control={
            <Switch
              aria-labelledby={thumbsLabel}
              checked={appearance.timeline_thumbs}
              onCheckedChange={(timeline_thumbs) => update({ timeline_thumbs })}
            />
          }
        />
      </SettingsGroup>
    </SettingsPage>
  );
}
