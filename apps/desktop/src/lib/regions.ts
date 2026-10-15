import type { Region } from "@/bindings/Region";

/**
 * Colour family of a region (`--color-region-<family>` tokens). Each HUD
 * region and manual boxes have their own; the screen surfaces share one.
 */
export type RegionFamily = "killfeed" | "banner" | "voice" | "chat" | "manual" | "screens";

/** Inspector group a region belongs to. */
export type RegionGroup = "hud" | "screens" | "manual";

/** Display data of one region. */
export interface RegionInfo {
  /** Human-readable name, always shown next to the colour. */
  readonly label: string;
  readonly family: RegionFamily;
  readonly group: RegionGroup;
}

/** Display data per region, in inspector order. */
export const REGIONS: Readonly<Record<Region, RegionInfo>> = {
  KillFeed: { label: "Kill feed", family: "killfeed", group: "hud" },
  Banner: { label: "Round banner", family: "banner", group: "hud" },
  VoiceChat: { label: "Voice chat", family: "voice", group: "hud" },
  TextChat: { label: "Text chat", family: "chat", group: "hud" },
  Scoreboard: { label: "Scoreboard", family: "screens", group: "screens" },
  DeathRecap: { label: "Death recap", family: "screens", group: "screens" },
  Spectating: { label: "Spectating", family: "screens", group: "screens" },
  MatchEnd: { label: "Match end", family: "screens", group: "screens" },
  Manual: { label: "Manual box", family: "manual", group: "manual" },
};

/** All regions in inspector order. */
export const REGION_ORDER = Object.keys(REGIONS) as readonly Region[];
