import { Fragment, useId } from "react";

import type { TrackSet } from "@/ipc/commands";
import { Checkbox } from "@/components/ui/checkbox";
import { RegionIcon } from "@/components/ui/region-chip";
import { Switch } from "@/components/ui/switch";
import {
  ALL_REGIONS,
  orderedRegions,
  type DetectedRegion,
  type RegionChoice,
} from "@/lib/job-settings";
import { REGIONS, type AnyRegion } from "@/lib/regions";

import { InspectorRow, InspectorSection, WordListInput } from "./parts";

/** What each region blurs (design 8.1 REGIONS). */
const DESCRIPTIONS: Readonly<Record<DetectedRegion, string>> = {
  KillFeed: "Killer and victim names in each kill feed row",
  Banner: "Player name in the round and ACE banners",
  VoiceChat: "Speaker name in the voice indicator, not the location",
  TextChat: "Name part of each chat line",
  Scoreboard: "Player names on the scoreboard (Tab)",
  DeathRecap: "Killer name on the death recap",
  Spectating: "Name in the “Spectating” banner",
  MatchEnd: "Names on the round end, MVP and match end screens",
};

const HUD = ALL_REGIONS.filter((region) => REGIONS[region].group === "hud");
const SCREENS = ALL_REGIONS.filter((region) => REGIONS[region].group === "screens");

export interface RegionsSectionProps {
  readonly regions: RegionChoice;
  /** Tracks of the analysed file, for the per-region counts. */
  readonly tracks: TrackSet | null;
  readonly onChange: (change: (regions: RegionChoice) => RegionChoice) => void;
}

/**
 * Inspector › Blur › Regions: "All regions" and one switch per region in
 * two groups, HUD and Screens, with the kill feed's keep-my-name options
 * and the text chat options under their rows. World-space name tags are
 * out of scope (D8), which the section says.
 */
export function RegionsSection({ regions, tracks, onChange }: RegionsSectionProps) {
  const enabled = new Set(regions.enabled);
  const allOn = ALL_REGIONS.every((region) => enabled.has(region));
  const noneOn = ALL_REGIONS.every((region) => !enabled.has(region));
  const allId = useId();
  const noteId = useId();

  const setRegion = (region: AnyRegion, on: boolean) =>
    onChange((current) => {
      const next = new Set(current.enabled);
      if (on) next.add(region);
      else next.delete(region);
      return { ...current, enabled: orderedRegions(next) };
    });

  const row = (region: DetectedRegion) => (
    <RegionRow
      key={region}
      region={region}
      description={DESCRIPTIONS[region]}
      on={enabled.has(region)}
      count={tracks ? tracks.tracks.filter((track) => track.region === region).length : null}
      onToggle={(on) => setRegion(region, on)}
    />
  );

  return (
    <InspectorSection title="Regions">
      <InspectorRow
        label={<label htmlFor={allId}>All regions</label>}
        description={
          <span id={noteId}>
            Name tags above players&apos; heads in the game world are out of scope and are never
            blurred.
          </span>
        }
        control={
          <Checkbox
            id={allId}
            aria-describedby={noteId}
            checked={allOn ? true : noneOn ? false : "indeterminate"}
            onCheckedChange={() =>
              onChange((current) => ({ ...current, enabled: allOn ? [] : ALL_REGIONS }))
            }
          />
        }
      />
      <GroupHeading>HUD</GroupHeading>
      {HUD.map((region) => (
        <Fragment key={region}>
          {row(region)}
          {region === "KillFeed" && enabled.has("KillFeed") && (
            <KeepNames regions={regions} onChange={onChange} />
          )}
          {region === "TextChat" && enabled.has("TextChat") && (
            <ChatOptions regions={regions} onChange={onChange} />
          )}
        </Fragment>
      ))}
      <GroupHeading>Screens</GroupHeading>
      {SCREENS.map(row)}
    </InspectorSection>
  );
}

/** A small caption that starts a group of region rows. */
function GroupHeading({ children }: { readonly children: string }) {
  return (
    <h3 className="border-t border-border bg-surface px-3 py-1 text-ui-xs font-medium text-foreground-subtle">
      {children}
    </h3>
  );
}

interface RegionRowProps {
  readonly region: AnyRegion;
  readonly description: string;
  readonly on: boolean;
  /** Tracks found in the region, `null` before an analysis. */
  readonly count: number | null;
  readonly onToggle: (on: boolean) => void;
}

/** One region: its icon, name, what it blurs, the number found and its switch. */
function RegionRow({ region, description, on, count, onToggle }: RegionRowProps) {
  const labelId = useId();
  const descriptionId = useId();
  return (
    <InspectorRow
      icon={<RegionIcon region={region} />}
      label={
        <span className="flex items-baseline gap-2">
          {REGIONS[region].label}
          {count !== null && (
            <span className="text-ui-xs font-normal text-foreground-subtle tabular-nums">
              {count === 0 ? "none found" : `${count} found`}
            </span>
          )}
        </span>
      }
      labelId={labelId}
      description={description}
      descriptionId={descriptionId}
      control={
        <Switch
          data-region={region}
          aria-labelledby={labelId}
          aria-describedby={descriptionId}
          checked={on}
          onCheckedChange={onToggle}
        />
      }
    />
  );
}

interface RegionOptionsProps {
  readonly regions: RegionChoice;
  readonly onChange: RegionsSectionProps["onChange"];
}

/** Kill feed: keep the own row or listed names readable (off by default, privacy first). */
function KeepNames({ regions, onChange }: RegionOptionsProps) {
  const ownId = useId();
  const namesId = useId();
  return (
    <>
      <InspectorRow
        nested
        label="Keep my own row readable"
        labelId={ownId}
        control={
          <Switch
            size="sm"
            aria-labelledby={ownId}
            checked={regions.keep_names.own_row}
            onCheckedChange={(own_row) =>
              onChange((current) => ({
                ...current,
                keep_names: { ...current.keep_names, own_row },
              }))
            }
          />
        }
      />
      <InspectorRow
        nested
        label={<label htmlFor={namesId}>Names to keep readable</label>}
        description="Comma separated. A name stays readable only where every read of it matches."
      >
        <WordListInput
          id={namesId}
          words={regions.keep_names.names}
          onChange={(names) =>
            onChange((current) => ({ ...current, keep_names: { ...current.keep_names, names } }))
          }
        />
      </InspectorRow>
    </>
  );
}

/** Text chat: also blur messages, and names in system lines. */
function ChatOptions({ regions, onChange }: RegionOptionsProps) {
  const messageId = useId();
  const systemId = useId();
  return (
    <>
      <InspectorRow
        nested
        label="Also blur message text"
        labelId={messageId}
        control={
          <Switch
            size="sm"
            aria-labelledby={messageId}
            checked={regions.chat_blur_message}
            onCheckedChange={(chat_blur_message) =>
              onChange((current) => ({ ...current, chat_blur_message }))
            }
          />
        }
      />
      <InspectorRow
        nested
        label="Blur names in system lines"
        labelId={systemId}
        control={
          <Switch
            size="sm"
            aria-labelledby={systemId}
            checked={regions.chat_blur_system}
            onCheckedChange={(chat_blur_system) =>
              onChange((current) => ({ ...current, chat_blur_system }))
            }
          />
        }
      />
    </>
  );
}
