/**
 * What the box overlay draws on a frame (design 8.1): the boxes the export
 * blurs, the leaks the last verification found there, the boxes the user
 * hid, and user edits that no longer match a detection (unbound, so
 * inactive). Pure data, so the overlay component only positions it.
 */
import type { NameKind } from "@/bindings/NameKind";
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";
import type { TrackSet } from "@/bindings/TrackSet";
import type { VerifyReport } from "@/bindings/VerifyReport";
import type { TrackBoxTarget } from "@/lib/edits";
import type { IntervalIndex } from "@/lib/interval-index";
import { formatCount } from "@/lib/media";
import { REGIONS } from "@/lib/regions";

/**
 * What editing an overlay item changes: a manual box (move, resize, set
 * its frames, delete), a detector box (move or resize = `Adjust`, delete
 * = `Suppress`) or another edit (delete only: shows a hidden box again,
 * drops an inactive edit).
 */
export type OverlayEdit =
  | { readonly kind: "manual"; readonly uid: string; readonly range: readonly [number, number] }
  | { readonly kind: "track"; readonly target: TrackBoxTarget }
  | { readonly kind: "edit"; readonly uid: string };

/** One thing drawn on the frame. */
export interface OverlayItem {
  /** Stable key across frames (track, edit or leak). */
  readonly key: string;
  /** A blurred box, a leak, an inactive edit or a box the user hid. */
  readonly kind: "box" | "leak" | "unbound" | "hidden";
  readonly rect: RectI;
  /** Region of a box or leak; `null` for a leak outside every region. */
  readonly region: Region | null;
  /** Short accessible name and tooltip ("Kill feed · victim · frames 4,312–4,401"). */
  readonly label: string;
  /** For leaks: whether it is still open (not fixed or accepted). */
  readonly open: boolean;
  /** How the item can be edited; `null` for leaks. */
  readonly edit: OverlayEdit | null;
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

/** The overlay items of `frame`: boxes, then hidden boxes and inactive edits, then leaks. */
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
    label: `${regionLabel(b.region)} · ${KIND_NAMES[b.kind]}${b.adjustUid === null ? "" : " (moved)"} · ${frames(b.span[0], b.span[1])}`,
    open: false,
    edit:
      b.uid !== null
        ? { kind: "manual", uid: b.uid, range: b.span }
        : {
            kind: "track",
            target: { region: b.region, span: b.span, source: b.source, adjustUid: b.adjustUid },
          },
  }));
  // Inactive edits, and the boxes the user hid (so they can be shown again).
  const edits: OverlayItem[] = tracks.overrides.flatMap((edit, i) => {
    if (edit.op === "Add" || edit.f0 > frame || edit.f1 < frame) return [];
    const active = index.active[i] ?? false;
    if (active && edit.op === "Adjust") return [];
    const where = `${regionLabel(edit.region)} · `;
    const label = active
      ? `${where}hidden box · ${frames(edit.f0, edit.f1)}`
      : `${where}${edit.op === "Suppress" ? "hidden box" : "moved box"} no longer matches a detection (inactive) · ${frames(edit.f0, edit.f1)}`;
    return [
      {
        key: `edit:${edit.uid}`,
        kind: active ? ("hidden" as const) : ("unbound" as const),
        rect: edit.anchor,
        region: edit.region,
        label,
        open: false,
        edit: { kind: "edit" as const, uid: edit.uid },
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
        edit: null,
      },
    ];
  });
  return [...boxes, ...edits, ...leaks];
}

/**
 * The frame N / Shift+N jumps to: the nearest start of a detection, a
 * user box or a leak that is not fixed after `frame` (`direction` 1) or
 * before it (-1); `null` when there is none.
 */
export function nextMark(
  frame: number,
  direction: 1 | -1,
  index: IntervalIndex,
  report: VerifyReport | null,
): number | null {
  const leaks = (report?.leaks ?? [])
    .filter((leak) => leak.status !== "Fixed")
    .map((leak) => leak.frame_range[0])
    .filter((start) => (direction === 1 ? start > frame : start < frame));
  const candidates = [index.nextStart(frame, direction), ...leaks].filter(
    (start): start is number => start !== null,
  );
  if (candidates.length === 0) return null;
  return direction === 1 ? Math.min(...candidates) : Math.max(...candidates);
}
