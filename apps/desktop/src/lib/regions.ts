import type { Region } from "@/bindings/Region";

/**
 * Screen surfaces outside the gameplay HUD whose names blurrd also blurs
 * (decision D8): scoreboard/tab, death recap, the "Spectating" banner and
 * the match-end/MVP/round-end screens. The engine adds them to the
 * generated `Region` type in milestone M13b; this local type covers them
 * until then and is replaced by the generated one.
 */
export type ScreenRegion = "Scoreboard" | "DeathRecap" | "Spectating" | "MatchEnd";

/** Every region the user interface can label. */
export type AnyRegion = Region | ScreenRegion;

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
export const REGIONS: Readonly<Record<AnyRegion, RegionInfo>> = {
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
export const REGION_ORDER = Object.keys(REGIONS) as readonly AnyRegion[];
