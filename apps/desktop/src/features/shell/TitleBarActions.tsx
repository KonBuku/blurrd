import { Search, Settings, SunMoon } from "lucide-react";

import type { Theme } from "@/bindings/Theme";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuLabel,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { useCommandStore } from "@/features/command/registry";
import { THEME_OPTIONS } from "@/features/settings/AppearanceSettings";
import { SETTINGS_BUTTON_ID } from "@/features/settings/SettingsView";
import { ariaKeys, SHORTCUTS } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useSettingsStore } from "@/store/settings";
import { useUiStore } from "@/store/ui";

const iconButton = "text-foreground-subtle hover:text-foreground";

function isTheme(value: string): value is Theme {
  return THEME_OPTIONS.some((option) => option.value === value);
}

/**
 * App actions in the title bar, left of the window controls: the command
 * palette, a quick theme menu and the settings toggle (design 8.1).
 */
export function TitleBarActions() {
  const theme = useSettingsStore((state) => state.appearance.theme);
  const updateAppearance = useSettingsStore((state) => state.updateAppearance);
  const settingsOpen = useUiStore((state) => state.view === "settings");
  const openSettings = useUiStore((state) => state.openSettings);
  const closeSettings = useUiStore((state) => state.closeSettings);
  const paletteOpen = useCommandStore((state) => state.open);
  const setPaletteOpen = useCommandStore((state) => state.setOpen);

  return (
    <div className="flex items-center gap-0.5">
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-md"
            aria-label="Command palette"
            aria-haspopup="dialog"
            aria-expanded={paletteOpen}
            aria-keyshortcuts={ariaKeys(SHORTCUTS.openCommandPalette)}
            className={iconButton}
            onClick={() => setPaletteOpen(true)}
          >
            <Search />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          Command palette
          <Kbd>{SHORTCUTS.openCommandPalette.keys.join("+")}</Kbd>
        </TooltipContent>
      </Tooltip>
      <DropdownMenu>
        <Tooltip>
          <TooltipTrigger asChild>
            <DropdownMenuTrigger asChild>
              <Button variant="ghost" size="icon-md" aria-label="Theme" className={iconButton}>
                <SunMoon />
              </Button>
            </DropdownMenuTrigger>
          </TooltipTrigger>
          <TooltipContent side="bottom">Theme</TooltipContent>
        </Tooltip>
        <DropdownMenuContent align="end" className="min-w-40">
          <DropdownMenuLabel>Theme</DropdownMenuLabel>
          <DropdownMenuRadioGroup
            value={theme}
            onValueChange={(value) => {
              if (isTheme(value)) updateAppearance({ theme: value });
            }}
          >
            {THEME_OPTIONS.map((option) => (
              <DropdownMenuRadioItem key={option.value} value={option.value}>
                {option.icon}
                {option.label}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            id={SETTINGS_BUTTON_ID}
            variant="ghost"
            size="icon-md"
            aria-label="Settings"
            aria-pressed={settingsOpen}
            className={cn(iconButton, "aria-pressed:bg-selected aria-pressed:text-foreground")}
            onClick={() => (settingsOpen ? closeSettings() : openSettings())}
          >
            <Settings />
          </Button>
        </TooltipTrigger>
        <TooltipContent side="bottom">
          Settings
          <Kbd>{SHORTCUTS.openSettings.keys.join("+")}</Kbd>
        </TooltipContent>
      </Tooltip>
    </div>
  );
}
