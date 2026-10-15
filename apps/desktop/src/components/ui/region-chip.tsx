import type { ComponentProps } from "react";

import type { Icon } from "@/components/icons/icon";
import {
  DeathRecapIcon,
  KillFeedIcon,
  ManualBoxIcon,
  MatchEndIcon,
  RoundBannerIcon,
  ScoreboardIcon,
  SpectatingIcon,
  TextChatIcon,
  VoiceChatIcon,
} from "@/components/icons/regions";
import type { Region } from "@/bindings/Region";
import { REGIONS, type RegionFamily } from "@/lib/regions";
import { cn } from "@/lib/utils";

/** Icon per region; the screen surfaces share a colour but not an icon. */
const REGION_ICONS: Readonly<Record<Region, Icon>> = {
  KillFeed: KillFeedIcon,
  Banner: RoundBannerIcon,
  VoiceChat: VoiceChatIcon,
  TextChat: TextChatIcon,
  Scoreboard: ScoreboardIcon,
  DeathRecap: DeathRecapIcon,
  Spectating: SpectatingIcon,
  MatchEnd: MatchEndIcon,
  Manual: ManualBoxIcon,
};

// Full class names, so that Tailwind finds them in the source.
const FAMILY_CLASSES: Readonly<Record<RegionFamily, string>> = {
  killfeed: "bg-region-killfeed-surface text-region-killfeed",
  banner: "bg-region-banner-surface text-region-banner",
  voice: "bg-region-voice-surface text-region-voice",
  chat: "bg-region-chat-surface text-region-chat",
  manual: "bg-region-manual-surface text-region-manual",
  screens: "bg-region-screens-surface text-region-screens",
};

// Full class names of the region colour per family, for icons on their own.
const FAMILY_TEXT: Readonly<Record<RegionFamily, string>> = {
  killfeed: "text-region-killfeed",
  banner: "text-region-banner",
  voice: "text-region-voice",
  chat: "text-region-chat",
  manual: "text-region-manual",
  screens: "text-region-screens",
};

export type RegionIconProps = {
  readonly region: Region;
  readonly className?: string;
};

/**
 * A region's icon in its colour, for rows that name the region in text
 * next to it (the inspector's region switches). Decorative.
 */
export function RegionIcon({ region, className }: RegionIconProps) {
  const Glyph = REGION_ICONS[region];
  return (
    <Glyph
      aria-hidden
      data-region={region}
      className={cn("size-4 shrink-0", FAMILY_TEXT[REGIONS[region].family], className)}
    />
  );
}

export type RegionChipProps = Omit<ComponentProps<"span">, "children"> & {
  readonly region: Region;
};

/**
 * A pill naming a region in its colour family, with its icon and label.
 * The label is always visible, so the colour is never the only cue.
 */
export function RegionChip({ region, className, ...props }: RegionChipProps) {
  const { label, family } = REGIONS[region];
  const Glyph = REGION_ICONS[region];
  return (
    <span
      data-slot="region-chip"
      data-region={region}
      className={cn(
        "inline-flex h-5 w-fit shrink-0 items-center gap-1 rounded-full px-2 text-ui-xs font-medium whitespace-nowrap",
        FAMILY_CLASSES[family],
        className,
      )}
      {...props}
    >
      <Glyph className="-ml-0.5 size-3" />
      {label}
    </span>
  );
}
