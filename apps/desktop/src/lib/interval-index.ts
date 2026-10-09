/**
 * Per-frame box lookup for the preview (plan 5.9): the boxes the export
 * blurs on a frame, computed from the project's tracks and overrides the
 * way `BlurPlan::boxes_at` does it (track boxes in track order with the
 * bound `Suppress`/`Adjust` edits applied, then the `Add` edits in list
 * order; Manual boxes ignore the region switches). Frames are bucketed so
 * a lookup touches only the tracks near the frame.
 */
import type { NameKind } from "@/bindings/NameKind";
import type { Override } from "@/bindings/Override";
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";
import type { Track } from "@/bindings/Track";

import type { BlurBox } from "./blur-geometry";

/** Minimum IoU of a `Suppress` edit with its anchor (`SUPPRESS_MIN_IOU`). */
export const SUPPRESS_MIN_IOU = 0.7;
/** Minimum IoU of an `Adjust` edit with its anchor (`ADJUST_MIN_IOU`). */
export const ADJUST_MIN_IOU = 0.5;

/** Frames per bucket of the lookup index. */
const BUCKET = 256;

/** A box of one frame with what the overlay shows about it. */
export interface FrameBox extends BlurBox {
  readonly region: Region;
  readonly kind: NameKind;
  /** The track, or `null` for a box the user added. */
  readonly trackId: number | null;
  /** The `Add` edit, or `null` for a track box. */
  readonly uid: string | null;
  /** First and last frame of the track or edit. */
  readonly span: readonly [number, number];
}

const MASK64 = (1n << 64n) - 1n;
const rotl64 = (v: bigint, n: bigint) => ((v << n) | (v >> (64n - n))) & MASK64;

/** The SplitMix64 finaliser folded to 32 bits (`mix32`). */
export function mix32(value: bigint): number {
  let z = (value + 0x9e3779b97f4a7c15n) & MASK64;
  z = ((z ^ (z >> 30n)) * 0xbf58476d1ce4e5b9n) & MASK64;
  z = ((z ^ (z >> 27n)) * 0x94d049bb133111ebn) & MASK64;
  return Number((z ^ (z >> 31n)) & 0xffffffffn);
}

/** Stable key of a region (`Region::key`), hashed into track seeds. */
const REGION_KEYS: Readonly<Record<Region, string>> = {
  KillFeed: "killfeed",
  Banner: "banner",
  VoiceChat: "voice",
  TextChat: "chat",
  Scoreboard: "scoreboard",
  DeathRecap: "recap",
  Spectating: "spectating",
  MatchEnd: "matchend",
  Manual: "manual",
};

/** The noise seed of a track (`Track::seed`). */
export function trackSeed(id: number, region: Region): number {
  let hash = 0n;
  for (const byte of new TextEncoder().encode(REGION_KEYS[region])) {
    hash = rotl64(hash, 8n) ^ BigInt(byte);
  }
  return mix32(BigInt(id) ^ rotl64(hash, 32n));
}

/** The noise seed of an `Add` edit, from its UUID. */
export function uidSeed(uid: string): number {
  const hex = uid.replaceAll("-", "");
  const value = /^[0-9a-f]{32}$/i.test(hex) ? BigInt(`0x${hex}`) : 0n;
  const high = value >> 64n;
  const low = value & MASK64;
  return mix32(high ^ rotl64(low, 32n));
}

/** Area of a rectangle. */
function area(r: RectI): number {
  return (r[2] - r[0]) * (r[3] - r[1]);
}

/** Intersection over union of two rectangles. */
export function iou(a: RectI, b: RectI): number {
  const w = Math.min(a[2], b[2]) - Math.max(a[0], b[0]);
  const h = Math.min(a[3], b[3]) - Math.max(a[1], b[1]);
  if (w <= 0 || h <= 0) return 0;
  const inter = w * h;
  return Math.fround(inter / (area(a) + area(b) - inter));
}

function frames(edit: Override): readonly [number, number] {
  return [edit.f0, edit.f1];
}

/** The edits that bind to a track, with their anchor and minimum IoU. */
function anchorOf(edit: Override): readonly [RectI, number] | null {
  if (edit.op === "Suppress") return [edit.anchor, SUPPRESS_MIN_IOU];
  if (edit.op === "Adjust") return [edit.anchor, ADJUST_MIN_IOU];
  return null;
}

/** Index of the first segment of `track` that ends at or after `frame`. */
function firstSegment(track: Track, frame: number): number {
  let lo = 0;
  let hi = track.segments.length;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if ((track.segments[mid]?.f1 ?? Infinity) < frame) lo = mid + 1;
    else hi = mid;
  }
  return lo;
}

/**
 * The boxes of a plan by frame. Build once per plan revision; `boxesAt`
 * is cheap enough to call on every presented video frame.
 */
export class IntervalIndex {
  readonly frameCount: number;
  private readonly tracks: readonly Track[];
  private readonly overrides: readonly Override[];
  /** Per track, the `Suppress`/`Adjust` edits bound to it. */
  private readonly bound: readonly (readonly number[])[];
  /** Per edit, whether it is active (`Bound`). */
  readonly active: readonly boolean[];
  /** Per bucket of {@link BUCKET} frames, the tracks that have boxes there. */
  private readonly buckets: readonly (readonly number[])[];
  private readonly seeds: readonly number[];
  private readonly addSeeds: readonly number[];
  /** Sorted first frames of every segment and edit, for "next detection". */
  private readonly starts: readonly number[];

  constructor(frameCount: number, tracks: readonly Track[], overrides: readonly Override[]) {
    this.frameCount = frameCount;
    this.tracks = tracks;
    this.overrides = overrides;
    const bound: number[][] = tracks.map(() => []);
    this.active = overrides.map((edit, index) => {
      const anchor = anchorOf(edit);
      if (!anchor) return true;
      const [f0, f1] = frames(edit);
      // The track of the edit's region with the highest IoU over the
      // edit's frames wins; ties go to the earlier track.
      let best = -1;
      let bestScore = 0;
      for (const [t, track] of tracks.entries()) {
        if (track.region !== edit.region) continue;
        let score = 0;
        for (let s = firstSegment(track, f0); s < track.segments.length; s += 1) {
          const segment = track.segments[s];
          if (!segment || segment.f0 > f1) break;
          score = Math.max(score, iou(segment.rect, anchor[0]));
        }
        if (score >= anchor[1] && (best === -1 || score > bestScore)) {
          best = t;
          bestScore = score;
        }
      }
      if (best === -1) return false;
      bound[best]?.push(index);
      return true;
    });
    this.bound = bound;
    const buckets: number[][] = Array.from(
      { length: Math.max(1, Math.ceil(frameCount / BUCKET)) },
      () => [],
    );
    tracks.forEach((track, t) => {
      let last = -1;
      for (const segment of track.segments) {
        const from = Math.max(Math.floor(segment.f0 / BUCKET), last + 1);
        const to = Math.min(Math.floor(segment.f1 / BUCKET), buckets.length - 1);
        for (let b = from; b <= to; b += 1) buckets[b]?.push(t);
        last = Math.max(last, to);
      }
    });
    this.buckets = buckets;
    this.seeds = tracks.map((track) => trackSeed(track.id, track.region));
    this.addSeeds = overrides.map((edit) => (edit.op === "Add" ? uidSeed(edit.uid) : 0));
    this.starts = [
      ...tracks.flatMap((track) => track.segments.map((s) => s.f0)),
      ...overrides.filter((edit) => edit.op === "Add").map((edit) => edit.f0),
    ].toSorted((a, b) => a - b);
  }

  /**
   * The boxes of `frame`, in the order the export composites them. Boxes
   * of regions not in `enabled` are skipped, except Manual boxes.
   */
  boxesAt(frame: number, enabled: ReadonlySet<Region>): FrameBox[] {
    const out: FrameBox[] = [];
    if (frame < 0 || frame >= this.frameCount) return out;
    const visible = (region: Region) => region === "Manual" || enabled.has(region);
    for (const t of this.buckets[Math.floor(frame / BUCKET)] ?? []) {
      const track = this.tracks[t];
      if (!track || !visible(track.region)) continue;
      const segment = track.segments[firstSegment(track, frame)];
      if (!segment || segment.f0 > frame) continue;
      const rect = this.apply(t, frame, segment.rect);
      if (!rect) continue;
      const anchor = track.segments[0]?.rect ?? segment.rect;
      out.push({
        rect,
        seed: this.seeds[t] ?? 0,
        gridOrigin: [anchor[0] + segment.text_offset[0], anchor[1] + segment.text_offset[1]],
        blockScale: ["PreRoll", "GapFill", "OcrFix"].includes(segment.conf) ? 1.5 : 1,
        region: track.region,
        kind: track.kind,
        trackId: track.id,
        uid: null,
        span: [track.segments[0]?.f0 ?? segment.f0, track.segments.at(-1)?.f1 ?? segment.f1],
      });
    }
    // Buckets list tracks in track order, so the order matches the export.
    this.overrides.forEach((edit, index) => {
      if (edit.op !== "Add" || edit.f0 > frame || edit.f1 < frame || !visible(edit.region)) return;
      out.push({
        rect: edit.rect,
        seed: this.addSeeds[index] ?? 0,
        gridOrigin: [edit.rect[0], edit.rect[1]],
        blockScale: 1,
        region: edit.region,
        kind: "Manual",
        trackId: null,
        uid: edit.uid,
        span: [edit.f0, edit.f1],
      });
    });
    return out;
  }

  /**
   * The first frame after `frame` (or before it, with `direction` -1) on
   * which a detection or user box starts, or `null`.
   */
  nextStart(frame: number, direction: 1 | -1): number | null {
    const { starts } = this;
    if (direction === 1) return starts.find((start) => start > frame) ?? null;
    return starts.findLast((start) => start < frame) ?? null;
  }

  /** Applies the edits bound to track `t` to its box on `frame`. */
  private apply(t: number, frame: number, rect: RectI): RectI | null {
    let out = rect;
    for (const index of this.bound[t] ?? []) {
      const edit = this.overrides[index];
      if (!edit || edit.f0 > frame || edit.f1 < frame) continue;
      if (edit.op === "Suppress" && iou(rect, edit.anchor) >= SUPPRESS_MIN_IOU) return null;
      if (edit.op === "Adjust" && iou(rect, edit.anchor) >= ADJUST_MIN_IOU) out = edit.rect;
    }
    return out;
  }
}
