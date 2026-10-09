/**
 * What the box overlay draws on a frame (design 8.1): the boxes the export
 * blurs, the leaks the last verification found there, and user edits that
 * no longer match a detection (unbound, so inactive). Pure data, so the
 * overlay component only positions it.
 */
import type { NameKind } from "@/bindings/NameKind";
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";
import type { VerifyReport } from "@/bindings/VerifyReport";
import type { TrackSet } from "@/ipc/commands";
import type { IntervalIndex } from "@/lib/interval-index";
import { formatCount } from "@/lib/media";
import { REGIONS } from "@/lib/regions";

/** One thing drawn on the frame. */
export interface OverlayItem {
  /** Stable key across frames (track, edit or leak). */
  readonly key: string;
  readonly kind: "box" | "leak" | "unbound";
  readonly rect: RectI;
  /** Region of a box or leak; `null` for a leak outside every region. */
  readonly region: Region | null;
  /** Short accessible name and tooltip ("Kill feed · victim · frames 4,312–4,401"). */
  readonly label: string;
  /** For leaks: whether it is still open (not fixed or accepted). */
  readonly open: boolean;
}

/** How a name kind is called in the tooltip. */
const KIND_NAMES: Readonly<Record<NameKind, string>> = {
  Killer: "killer",
  Victim: "victim",
  Banner: "player name",
  Speaker: "speaker",
  ChatSender: "sender",
  ScoreboardRow: "scoreboard row",
  RecapName: "player name",
  Spectated: "spectated player",
  MatchEndName: "player name",
  Manual: "manual box",
  OcrFix: "found by OCR",
};

function frames(f0: number, f1: number): string {
  return f0 === f1 ? `frame ${formatCount(f0)}` : `frames ${formatCount(f0)}–${formatCount(f1)}`;
}

function regionLabel(region: Region | null): string {
  return region === null ? "Outside the HUD" : REGIONS[region].label;
}

/** The overlay items of `frame`, boxes first, then unbound edits, then leaks. */
export function overlayItems(
  frame: number,
  index: IntervalIndex,
  enabled: ReadonlySet<Region>,
  tracks: TrackSet,
  report: VerifyReport | null,
): OverlayItem[] {
  const boxes: OverlayItem[] = index.boxesAt(frame, enabled).map((b) => ({
    key: b.trackId === null ? `edit:${b.uid ?? ""}` : `track:${b.trackId}`,
    kind: "box",
    rect: b.rect,
    region: b.region,
    label: `${regionLabel(b.region)} · ${KIND_NAMES[b.kind]} · ${frames(b.span[0], b.span[1])}`,
    open: false,
  }));
  const unbound: OverlayItem[] = tracks.overrides.flatMap((edit, i) => {
    if (edit.op === "Add" || index.active[i] || edit.f0 > frame || edit.f1 < frame) return [];
    const what = edit.op === "Suppress" ? "hidden box" : "moved box";
    return [
      {
        key: `edit:${edit.uid}`,
        kind: "unbound" as const,
        rect: edit.anchor,
        region: edit.region,
        label: `${regionLabel(edit.region)} · ${what} no longer matches a detection (inactive) · ${frames(edit.f0, edit.f1)}`,
        open: false,
      },
    ];
  });
  const leaks: OverlayItem[] = (report?.leaks ?? []).flatMap((leak, i) => {
    const [f0, f1] = leak.frame_range;
    if (f0 > frame || f1 < frame || leak.status === "Fixed") return [];
    const open = leak.status === "Open";
    const what = leak.kind === "OutOfScope" ? "name outside the blurred regions" : "readable name";
    const state = open ? "open" : leak.status === "Accepted" ? "accepted" : "information";
    return [
      {
        key: `leak:${i}`,
        kind: "leak" as const,
        rect: leak.rect,
        region: leak.region,
        label: `${regionLabel(leak.region)} · ${what} (${state}) · ${frames(f0, f1)}`,
        open,
      },
    ];
  });
  return [...boxes, ...unbound, ...leaks];
}
