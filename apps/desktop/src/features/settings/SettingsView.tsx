import {
  ChevronRight,
  Cpu,
  FileOutput,
  Info,
  Keyboard,
  Palette,
  ScanSearch,
  ShieldCheck,
  SlidersHorizontal,
  X,
  ScrollText,
} from "lucide-react";
import { useEffect, useRef, type ComponentType, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { Frame, FrameHeader } from "@/features/shell/Frame";
import { SHORTCUTS } from "@/lib/shortcuts";
import { cn } from "@/lib/utils";
import { useUiStore, type SettingsSection } from "@/store/ui";

import { AboutSettings } from "./AboutSettings";
import { AppearanceSettings } from "./AppearanceSettings";
import { DetectionSettings } from "./DetectionSettings";
import { GeneralSettings } from "./GeneralSettings";
import { OutputSettings } from "./OutputSettings";
import { PrivacySettings } from "./PrivacySettings";
import { ProcessingSettings } from "./ProcessingSettings";
import { ShortcutSettings } from "./ShortcutSettings";
import { VerificationSettings } from "./VerificationSettings";

/** Id of the title bar button that opens the settings. */
export const SETTINGS_BUTTON_ID = "settings-button";

/** The settings pages, in navigation order. */
const SECTIONS: readonly {
  id: SettingsSection;
  label: string;
  icon: ReactNode;
  page: ComponentType;
}[] = [
  {
    id: "general",
    label: "General",
    icon: <SlidersHorizontal aria-hidden />,
    page: GeneralSettings,
  },
  {
    id: "appearance",
    label: "Appearance",
    icon: <Palette aria-hidden />,
    page: AppearanceSettings,
  },
  { id: "processing", label: "Processing", icon: <Cpu aria-hidden />, page: ProcessingSettings },
  {
    id: "output",
    label: "Output defaults",
    icon: <FileOutput aria-hidden />,
    page: OutputSettings,
  },
  {
    id: "detection",
    label: "Detection",
    icon: <ScanSearch aria-hidden />,
    page: DetectionSettings,
  },
  {
    id: "verification",
    label: "Verification",
    icon: <ShieldCheck aria-hidden />,
    page: VerificationSettings,
  },
  { id: "shortcuts", label: "Shortcuts", icon: <Keyboard aria-hidden />, page: ShortcutSettings },
  {
    id: "privacy",
    label: "Logs & privacy",
    icon: <ScrollText aria-hidden />,
    page: PrivacySettings,
  },
  { id: "about", label: "About", icon: <Info aria-hidden />, page: AboutSettings },
];

/**
 * The settings view (design 8.5). It replaces the preview and inspector
 * frames: a breadcrumb header with a close button, the page navigation on
 * the left and the selected page in a centred column. Escape or the close
 * button returns to the workspace; focus starts on the selected page link
 * and goes back to the settings button on close.
 */
export function SettingsView() {
  const section = useUiStore((state) => state.settingsSection);
  const openSettings = useUiStore((state) => state.openSettings);
  const closeSettings = useUiStore((state) => state.closeSettings);
  const activeLink = useRef<HTMLButtonElement>(null);
  const current = SECTIONS.find((candidate) => candidate.id === section) ?? SECTIONS[0];

  useEffect(() => {
    activeLink.current?.focus();
    return () => {
      // After the view unmounts, focus would fall back to <body>.
      requestAnimationFrame(() => {
        if (document.activeElement === document.body) {
          document.getElementById(SETTINGS_BUTTON_ID)?.focus();
        }
      });
    };
  }, []);

  if (!current) return null;
  const Page = current.page;

  return (
    <Frame aria-label="Settings" className="flex-1">
      <FrameHeader>
        <nav aria-label="Breadcrumb" className="min-w-0 flex-1">
          <ol className="flex items-center gap-1 text-ui-base">
            <li className="text-foreground-subtle">Settings</li>
            <li aria-hidden className="text-foreground-subtle">
              <ChevronRight className="size-3.5" />
            </li>
            <li aria-current="page" className="truncate font-medium">
              {current.label}
            </li>
          </ol>
        </nav>
        <Tooltip>
          <TooltipTrigger asChild>
            <Button
              variant="ghost"
              size="icon-md"
              aria-label="Close settings"
              className="text-foreground-subtle hover:text-foreground"
              onClick={closeSettings}
            >
              <X />
            </Button>
          </TooltipTrigger>
          <TooltipContent side="bottom">
            Close settings
            <Kbd>{SHORTCUTS.closeOverlay.keys.join("+")}</Kbd>
          </TooltipContent>
        </Tooltip>
      </FrameHeader>
      <div className="flex min-h-0 flex-1">
        <nav aria-label="Settings pages" className="w-50 shrink-0 overflow-y-auto p-2">
          <ul className="flex flex-col gap-0.5">
            {SECTIONS.map(({ id, label, icon }) => (
              <li key={id}>
                <button
                  ref={id === section ? activeLink : undefined}
                  type="button"
                  aria-current={id === section ? "page" : undefined}
                  className={cn(
                    "flex h-8 w-full cursor-default items-center gap-2 rounded-lg px-2.5 text-ui-base text-foreground-subtle transition-colors hover:bg-hover hover:text-foreground [&_svg]:size-4 [&_svg]:shrink-0",
                    id === section && "bg-selected text-foreground",
                  )}
                  onClick={() => openSettings(id)}
                >
                  {icon}
                  {label}
                </button>
              </li>
            ))}
          </ul>
        </nav>
        {/* Focusable, so that a page without controls (Shortcuts) still scrolls by keyboard. */}
        {/* Keyed by page, so every page opens at its top. */}
        <div
          key={current.id}
          // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
          tabIndex={0}
          className="min-w-0 flex-1 overflow-y-auto outline-none [scrollbar-gutter:stable] focus-visible:ring-2 focus-visible:ring-ring focus-visible:ring-inset"
        >
          <div className="mx-auto w-full max-w-4xl px-4 pt-6 pb-10 lg:px-8">
            <Page />
          </div>
        </div>
      </div>
    </Frame>
  );
}
