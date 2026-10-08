import { Download, FolderOpen, Play, Plus, Settings, Trash2 } from "lucide-react";

import { BlurStyleIcon, GpuIcon, OcrVerifyIcon, PixelateIcon } from "@/components/icons/processing";
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
import { MaximizeIcon, RestoreIcon } from "@/components/icons/window";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { RegionChip } from "@/components/ui/region-chip";
import { REGION_ORDER, REGIONS } from "@/lib/regions";

import { Section, Specimen } from "./Section";

const SURFACES = [
  ["background-win-alt", "bg-background-win-alt"],
  ["background", "bg-background"],
  ["panel", "bg-panel"],
  ["card", "bg-card"],
  ["surface", "bg-surface"],
  ["secondary", "bg-secondary"],
  ["primary", "bg-primary"],
  ["accent", "bg-accent"],
  ["accent-strong", "bg-accent-strong"],
  ["success", "bg-success"],
  ["warning", "bg-warning"],
  ["destructive", "bg-destructive"],
] as const;

const LUCIDE_ICONS = [
  ["Play", Play],
  ["Add", Plus],
  ["Open folder", FolderOpen],
  ["Download", Download],
  ["Settings", Settings],
  ["Delete", Trash2],
] as const;

const ICONS = [
  ["KillFeed", KillFeedIcon],
  ["RoundBanner", RoundBannerIcon],
  ["VoiceChat", VoiceChatIcon],
  ["TextChat", TextChatIcon],
  ["Scoreboard", ScoreboardIcon],
  ["DeathRecap", DeathRecapIcon],
  ["Spectating", SpectatingIcon],
  ["MatchEnd", MatchEndIcon],
  ["ManualBox", ManualBoxIcon],
  ["BlurStyle", BlurStyleIcon],
  ["Pixelate", PixelateIcon],
  ["OcrVerify", OcrVerifyIcon],
  ["Gpu", GpuIcon],
  ["Maximize", MaximizeIcon],
  ["Restore", RestoreIcon],
] as const;

/** Colours, type scale, icons, buttons, badges and region chips. */
export function FoundationSections() {
  return (
    <>
      <Section title="Surfaces and colours">
        {SURFACES.map(([name, className]) => (
          <Specimen key={name} label={name}>
            <div className={`size-12 rounded-lg border border-border ${className}`} />
          </Specimen>
        ))}
      </Section>

      <Section title="Type scale" className="flex-col items-start gap-1">
        <p className="text-ui-xl font-semibold">Heading xl, 18 px semibold</p>
        <p className="text-ui-lg font-medium">Heading lg, 16 px medium</p>
        <p className="text-ui-base">Body base, 14 px: Blur player names in gameplay recordings.</p>
        <p className="text-ui-base text-foreground-subtle">
          Secondary text at 60 % of the primary colour.
        </p>
        <p className="text-ui-base text-foreground-subtlest">Tertiary text and placeholders.</p>
        <p className="text-ui-sm text-foreground-subtle">Small, 12 px helper text.</p>
        <p className="text-ui-xs tracking-label text-foreground-subtlest uppercase">
          Extra small, 10 px label
        </p>
        <p className="font-mono text-ui-sm tabular-nums">00:01:23.456 · frame 4 987 · h264_nvenc</p>
        <a href="#gallery" className="text-ui-base text-link underline-offset-4 hover:underline">
          Text link
        </a>
      </Section>

      <Section title="Icons (Lucide and blurrd)">
        {LUCIDE_ICONS.map(([name, LucideIcon]) => (
          <LucideIcon key={name} className="size-4" aria-label={name} />
        ))}
        <span className="h-4 w-px bg-border" />
        {ICONS.map(([name, BlurrdIcon]) => (
          <BlurrdIcon key={name} title={name} />
        ))}
      </Section>

      <Section title="Buttons">
        <Button>Export 3 files</Button>
        <Button variant="secondary">Analyze</Button>
        <Button variant="outline">Browse</Button>
        <Button variant="ghost">Cancel</Button>
        <Button variant="destructive">Delete</Button>
        <Button variant="warning">Accept unverified</Button>
        <Button variant="link">Open log folder</Button>
        <Button disabled>Disabled</Button>
      </Section>

      <Section title="Button sizes and icon buttons">
        <Button size="xs">Extra small</Button>
        <Button size="sm">Small</Button>
        <Button size="md">
          <Plus />
          Add files
        </Button>
        <Button size="lg">Large</Button>
        <Button variant="ghost" size="icon-xs" aria-label="Add (extra small)">
          <Plus />
        </Button>
        <Button variant="ghost" size="icon-sm" aria-label="Add (small)">
          <Plus />
        </Button>
        <Button variant="ghost" size="icon-md" aria-label="Settings">
          <Settings />
        </Button>
        <Button variant="outline" size="icon-lg" aria-label="Open folder">
          <FolderOpen />
        </Button>
      </Section>

      <Section title="Badges and shortcuts">
        <Badge>New</Badge>
        <Badge variant="secondary">12 boxes</Badge>
        <Badge variant="outline">H.264</Badge>
        <Badge variant="success">Clean</Badge>
        <Badge variant="warning">Unverified</Badge>
        <Badge variant="destructive">2 leaks</Badge>
        <Kbd>Ctrl</Kbd>
        <Kbd>K</Kbd>
        <Kbd>Space</Kbd>
      </Section>

      <Section title="Region chips">
        {(["hud", "screens", "manual"] as const).map((group) => (
          <div key={group} className="flex flex-wrap items-center gap-2">
            {REGION_ORDER.filter((region) => REGIONS[region].group === group).map((region) => (
              <RegionChip key={region} region={region} />
            ))}
          </div>
        ))}
      </Section>
    </>
  );
}
