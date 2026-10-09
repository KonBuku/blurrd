import {
  ChevronDown,
  FastForward,
  Pause,
  Play,
  Rewind,
  SkipBack,
  SkipForward,
  StepBack,
  StepForward,
  Volume2,
  VolumeX,
} from "lucide-react";
import type { ReactNode } from "react";

import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { Kbd } from "@/components/ui/kbd";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { formatTimecode } from "@/lib/frames";
import { formatCount } from "@/lib/media";
import { SHORTCUTS, ariaKeys, shortcutText, type ShortcutId } from "@/lib/shortcuts";

import { RATES, useControllerStore, usePlaybackStore } from "./playback";
import type { LoadedProject } from "./useProject";

/** Label of a playback speed ("0.25×"). */
export function rateLabel(rate: number): string {
  return `${rate}×`;
}

interface TransportButtonProps {
  readonly label: string;
  /** The shortcut shown in the tooltip and announced to assistive technology. */
  readonly shortcut: ShortcutId;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly children: ReactNode;
}

/** An icon button of the transport with its shortcut in the tooltip. */
function TransportButton({ label, shortcut, onClick, disabled, children }: TransportButtonProps) {
  const keys = SHORTCUTS[shortcut];
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-md"
          aria-label={label}
          aria-keyshortcuts={ariaKeys(keys)}
          disabled={disabled}
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">
        {label}
        <Kbd>{shortcutText(keys)}</Kbd>
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * The position text: timecode and duration, or the 0-based decoded frame
 * index and the last index. The only part of the transport that changes
 * on every frame, so it subscribes to the frame on its own.
 */
function PositionText({
  data,
  frameNumbers,
}: {
  readonly data: LoadedProject;
  readonly frameNumbers: boolean;
}) {
  const { clock } = data;
  const frame = usePlaybackStore((state) => state.target ?? state.frame);
  const last = Math.max(clock.count - 1, 0);
  return (
    <span data-testid="transport-position" data-frame={frame}>
      {frameNumbers ? (
        <>
          <span className="sr-only">Frame </span>
          {formatCount(frame)} / {formatCount(last)}
        </>
      ) : (
        `${formatTimecode(clock.startOf(frame))} / ${formatTimecode(clock.duration)}`
      )}
    </span>
  );
}

/** The position of the shown frame; a click switches between time and frame numbers. */
function PositionButton({ data }: { readonly data: LoadedProject }) {
  const showFrameNumber = usePlaybackStore((state) => state.showFrameNumber);
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="sm"
          className="ml-1 font-mono tabular-nums"
          onClick={() => usePlaybackStore.setState({ showFrameNumber: !showFrameNumber })}
        >
          <PositionText data={data} frameNumbers={showFrameNumber} />
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">
        {showFrameNumber
          ? "Frame number (the first frame is 0) and the last frame · click for the timecode"
          : "Timecode and duration · click for frame numbers"}
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * The transport under the stage (design 8.1): first / previous frame /
 * play-pause / next frame / last, the timecode of the presented frame
 * and the duration (click switches to frame numbers: the 0-based decoded
 * frame index and the last index), the J / L shuttle speed while it runs,
 * the speed menu and the preview audio. Every move goes through the
 * playback controller, so the shown frame is always a presented one.
 */
export function Transport({ data }: { readonly data: LoadedProject }) {
  const { clock, project } = data;
  const controller = useControllerStore((state) => state.controller);
  const playing = usePlaybackStore((state) => state.playing);
  const shuttle = usePlaybackStore((state) => state.shuttle);
  const rate = usePlaybackStore((state) => state.rate);
  const muted = usePlaybackStore((state) => state.muted);
  const last = Math.max(clock.count - 1, 0);
  const hasAudio = project.media.audio.length > 0;
  const off = controller === null;

  return (
    <div
      role="toolbar"
      aria-label="Transport"
      className="flex h-10 shrink-0 items-center gap-1 border-t border-border px-2"
      data-testid="transport"
    >
      <TransportButton
        label="Go to the first frame"
        shortcut="goToStart"
        disabled={off}
        onClick={() => controller?.seek(0)}
      >
        <SkipBack />
      </TransportButton>
      <TransportButton
        label="Previous frame"
        shortcut="previousFrame"
        disabled={off}
        onClick={() => controller?.step(-1)}
      >
        <StepBack />
      </TransportButton>
      <TransportButton
        label={playing ? "Pause" : "Play"}
        shortcut="playPause"
        disabled={off}
        onClick={() => controller?.togglePlay()}
      >
        {playing ? <Pause /> : <Play />}
      </TransportButton>
      <TransportButton
        label="Next frame"
        shortcut="nextFrame"
        disabled={off}
        onClick={() => controller?.step(1)}
      >
        <StepForward />
      </TransportButton>
      <TransportButton
        label="Go to the last frame"
        shortcut="goToEnd"
        disabled={off}
        onClick={() => controller?.seek(last)}
      >
        <SkipForward />
      </TransportButton>
      <PositionButton data={data} />
      {playing && shuttle !== 1 && (
        <Badge className="gap-1 tabular-nums" data-testid="shuttle">
          {shuttle < 0 ? (
            <Rewind aria-hidden className="size-3" />
          ) : (
            <FastForward aria-hidden className="size-3" />
          )}
          <span className="sr-only">{shuttle < 0 ? "Backwards" : "Forwards"}</span>
          {rateLabel(Math.abs(shuttle))}
        </Badge>
      )}
      <div className="flex-1" />
      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button
            variant="ghost"
            size="sm"
            aria-label={`Speed: ${rateLabel(rate)}`}
            className="tabular-nums"
          >
            {rateLabel(rate)}
            <ChevronDown />
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end">
          <DropdownMenuRadioGroup
            value={String(rate)}
            onValueChange={(value) => controller?.setRate(Number(value))}
          >
            {RATES.map((r) => (
              <DropdownMenuRadioItem key={r} value={String(r)}>
                {rateLabel(r)}
              </DropdownMenuRadioItem>
            ))}
          </DropdownMenuRadioGroup>
        </DropdownMenuContent>
      </DropdownMenu>
      <Tooltip>
        <TooltipTrigger asChild>
          <Button
            variant="ghost"
            size="icon-md"
            aria-label="Preview audio"
            aria-pressed={!muted && hasAudio}
            // Not `disabled`: the tooltip says why there is nothing to hear.
            aria-disabled={!hasAudio || undefined}
            disabled={off}
            className="aria-disabled:opacity-50"
            onClick={() => {
              if (hasAudio) controller?.setMuted(!muted);
            }}
          >
            {muted || !hasAudio ? <VolumeX /> : <Volume2 />}
          </Button>
        </TooltipTrigger>
        <TooltipContent side="top">
          {hasAudio
            ? muted
              ? "Play the preview audio"
              : "Mute the preview audio"
            : "This recording has no audio"}
        </TooltipContent>
      </Tooltip>
    </div>
  );
}
