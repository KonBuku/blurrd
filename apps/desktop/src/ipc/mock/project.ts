/**
 * Projects of the mock backend: every opened file plays one of the
 * synthetic fixture videos (`fixture-media.ts`) with tracks that follow
 * its name plates, a Manual box on the alignment target, an unbound edit,
 * a health span and a verification report with leaks. Paths containing
 * "hevc" play the HEVC fixture with a VP9 proxy (the fallback path);
 * paths containing "fresh" are not analysed yet.
 */
import type { DensityLanes } from "@/bindings/DensityLanes";
import type { FramePts } from "@/bindings/FramePts";
import type { Override } from "@/bindings/Override";
import type { ProjectSummary } from "@/bindings/ProjectSummary";
import type { Region } from "@/bindings/Region";
import type { Segment } from "@/bindings/Segment";
import type { Track } from "@/bindings/Track";
import type { TrackSet } from "@/bindings/TrackSet";
import type { VerifyReport } from "@/bindings/VerifyReport";
import { IntervalIndex, overrideBindings } from "@/lib/interval-index";

import {
  FIXTURE,
  FIXTURE_URLS,
  PLATES,
  TARGET_RECT,
  THUMB_EVERY,
  plateRect,
} from "./fixture-media";
import { fakeMedia } from "./media";

/** The user box on the alignment target (always bound: it adds a box). */
export const MANUAL_UID = "0f8c3a52-6d0e-4b8e-9c1a-2a5d7b3e9f10";
/** A hidden-box edit whose anchor matches no track (shown as unbound). */
export const UNBOUND_UID = "7d2e4f61-0b9a-4c3d-8e5f-1a2b3c4d5e6f";

/** Kind of name per region, for the fixture tracks. */
const KINDS = {
  KillFeed: ["Killer", "Victim"],
  Banner: ["Banner"],
  TextChat: ["ChatSender"],
} as const;

/** A short stable id for a path (FNV-1a, hex). */
export function projectId(path: string): string {
  let value = 0x811c9dc5;
  for (const char of path.toLowerCase()) {
    value ^= char.codePointAt(0) ?? 0;
    value = Math.imul(value, 0x01000193) >>> 0;
  }
  return value.toString(16).padStart(8, "0");
}

/** Frames of the video a path plays. */
export function frameCount(path: string): number {
  return path.toLowerCase().includes("hevc") ? 60 : FIXTURE.frames;
}

/** The project a mock `open_project` returns for `path`. */
export function mockProject(path: string): ProjectSummary {
  const hevc = path.toLowerCase().includes("hevc");
  const frames = frameCount(path);
  const media = {
    ...fakeMedia(path),
    width: FIXTURE.width,
    height: FIXTURE.height,
    fps: [FIXTURE.fps, 1] as [number, number],
    time_base: [...FIXTURE.timeBase] as [number, number],
    frame_count: frames,
    duration_s: frames / FIXTURE.fps,
    video_codec: hevc ? "hevc" : "vp9",
  };
  return {
    id: projectId(path),
    media,
    media_url: hevc ? FIXTURE_URLS.hevc : FIXTURE_URLS.video,
    proxy_url: hevc ? FIXTURE_URLS.proxy : null,
    sy: FIXTURE.height / 1080,
    blur: {
      style: "PixelBlur",
      strength: "Medium",
      pad: [4, 3],
      radius: 6,
      feather: 1.5,
      noise: true,
    },
    enabled: ["KillFeed", "Banner", "VoiceChat", "TextChat"],
    analyzed: initiallyAnalyzed(path),
    verify: {
      plan_revision: 3,
      state: "LeaksRemain",
      unverified_ranges: [[Math.min(150, frames - 1), Math.min(170, frames - 1)]],
      report: "verify-1.json",
    },
    thumbs: { every: THUMB_EVERY, count: Math.ceil(frames / THUMB_EVERY) },
  };
}

/** Presentation timestamps of the fixture: WebM stores whole milliseconds. */
export function mockPts(path: string): FramePts {
  const pts = Array.from({ length: frameCount(path) }, (_, i) =>
    Math.round((i * 1000) / FIXTURE.fps),
  );
  return { time_base: [...FIXTURE.timeBase] as [number, number], pts };
}

/** The tracks of the fixture's plates, cut to `frames`. */
function plateTracks(frames: number): Track[] {
  const perRegion = new Map<Region, number>();
  return PLATES.flatMap((plate, index) => {
    const nth = perRegion.get(plate.region) ?? 0;
    perRegion.set(plate.region, nth + 1);
    const kinds: readonly Track["kind"][] = KINDS[plate.region as keyof typeof KINDS] ?? ["Manual"];
    const segments: Segment[] = [];
    const last = Math.min(plate.f1, frames - 1);
    const anchor = plateRect(plate, plate.f0);
    if (!anchor || plate.f0 > last) return [];
    // Sliding frames get one segment each; the grid follows the text.
    for (let f = plate.f0; f <= last; f += 1) {
      const rect = plateRect(plate, f);
      if (!rect) continue;
      const previous = segments.at(-1);
      if (previous && previous.rect.every((v, i) => v === rect[i])) {
        segments[segments.length - 1] = { ...previous, f1: f };
      } else {
        const sliding = f < plate.f0 + plate.slide;
        segments.push({
          f0: f,
          f1: f,
          rect,
          conf: sliding ? "PreRoll" : "Tracked",
          text_offset: [rect[0] - anchor[0], rect[1] - anchor[1]],
        });
      }
    }
    return [
      {
        id: index + 1,
        region: plate.region,
        kind: kinds[nth % kinds.length] ?? "Manual",
        source: "Detector" as const,
        segments,
      },
    ];
  });
}

/** The mock edits: a Manual box on the target and an unbound hidden box. */
function mockOverrides(frames: number): Override[] {
  return [
    { op: "Add", uid: MANUAL_UID, region: "Manual", f0: 0, f1: frames - 1, rect: TARGET_RECT },
    {
      op: "Suppress",
      uid: UNBOUND_UID,
      region: "KillFeed",
      f0: 20,
      f1: Math.min(90, frames - 1),
      anchor: [380, 200, 440, 214],
    },
  ];
}

/** Whether the mock opens `path` analysed (paths with "fresh" are not). */
export function initiallyAnalyzed(path: string): boolean {
  return !path.toLowerCase().includes("fresh");
}

/**
 * The initial `get_tracks` answer of the mock for `path` (before any
 * edit), or the tracks of its first analysis when `analyzed` is set.
 */
export function mockTracks(path: string, analyzed = initiallyAnalyzed(path)): TrackSet {
  const frames = frameCount(path);
  if (!analyzed) {
    return { revision: 0, tracks: [], overrides: [], bindings: [], health: [] };
  }
  const tracks = plateTracks(frames);
  const overrides = mockOverrides(frames);
  return {
    revision: 3,
    tracks,
    overrides,
    bindings: overrideBindings(tracks, overrides),
    health: [{ region: "TextChat", f0: Math.min(180, frames - 1), f1: Math.min(200, frames - 1) }],
  };
}

/**
 * `get_timeline_density` of the track set `stored` (by default the fixture's
 * initial one), computed like `BlurPlan::density`.
 */
export function mockDensity(
  path: string,
  buckets: number,
  stored: TrackSet = mockTracks(path),
): DensityLanes {
  const { tracks, overrides } = stored;
  const frames = frameCount(path);
  const count = Math.min(buckets, frames);
  const index = new IntervalIndex(frames, tracks, overrides);
  const all = new Set<Region>(["KillFeed", "Banner", "VoiceChat", "TextChat", "Manual"]);
  const perFrame = Array.from(
    { length: frames },
    (_, f) => new Set(index.boxesAt(f, all).map((b) => b.region)),
  );
  const regions = (["KillFeed", "Banner", "VoiceChat", "TextChat", "Manual"] as const).filter((r) =>
    perFrame.some((set) => set.has(r)),
  );
  const at = (b: number) => Math.floor((b * frames) / count);
  return {
    frame_count: frames,
    buckets: count,
    lanes: regions.map((region) => ({
      region,
      coverage: Array.from({ length: count }, (_, b) => {
        const slice = perFrame.slice(at(b), at(b + 1));
        return slice.filter((set) => set.has(region)).length / slice.length;
      }),
    })),
  };
}

/**
 * `get_verify_report`: one open leak, one fixed leak and an out-of-scope
 * name; `null` for a file that was never analysed unless `revision` (the
 * revision an analysis verified) is given.
 */
export function mockVerifyReport(path: string, revision?: number): VerifyReport | null {
  if (revision === undefined && !initiallyAnalyzed(path)) return null;
  const frames = frameCount(path);
  const clamp = (f: number) => Math.min(f, frames - 1);
  return {
    schema: "blurrd-verify/1",
    version: 1,
    plan_revision: revision ?? 3,
    source_fingerprint: { size: 1, mtime: 0, blake3_head_tail: "0".repeat(64) },
    output_sha256: null,
    coverage: "Structure",
    frames_checked: frames,
    per_region: { KillFeed: { positive_control: 0.98, items: 4, unverifiable_items: 0 } },
    leaks: [
      {
        frame_range: [clamp(30), clamp(34)],
        region: "KillFeed",
        zone: "NameSlot",
        rect: [470, 40, 540, 52],
        conf: 0.81,
        kind: "Dictionary",
        status: "Open",
        dedup: null,
      },
      {
        frame_range: [clamp(120), clamp(121)],
        region: "Banner",
        zone: "NameSlot",
        rect: [270, 150, 370, 168],
        conf: 0.66,
        kind: "Dictionary",
        status: "Fixed",
        dedup: null,
      },
      {
        frame_range: [clamp(40), clamp(44)],
        region: null,
        zone: "FullFrame",
        rect: [180, 100, 240, 112],
        conf: 0.7,
        kind: "OutOfScope",
        status: "Info",
        dedup: null,
      },
    ],
    summary: {
      state: "LeaksRemain",
      open: 1,
      fixed: 1,
      accepted: 0,
      out_of_scope: 1,
      unverifiable_items: 0,
    },
  };
}
