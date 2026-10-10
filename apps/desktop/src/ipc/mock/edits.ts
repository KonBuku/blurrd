/**
 * Stored edits of the mock backend: per opened file the track set, its
 * revision and the frame ranges edits changed since the last verification.
 * `set_overrides` behaves like the engine's command (plan 9.1, 6.1): it
 * rejects a stale `base_rev`, checks every edit, stores the full list as
 * a new revision and marks the frames of every changed edit unverified.
 * An analysis replaces the tracks with those of the enabled regions and
 * verifies the whole video again.
 */
import type { AppError } from "@/bindings/AppError";
import type { FrameRange } from "@/bindings/FrameRange";
import type { Override } from "@/bindings/Override";
import type { RectI } from "@/bindings/RectI";
import type { Region } from "@/bindings/Region";
import type { Leak } from "@/bindings/Leak";
import type { VerifyReport } from "@/bindings/VerifyReport";
import type { VerifySummary } from "@/bindings/VerifySummary";
import type { OverridesSaved, TrackSet } from "@/ipc/commands";
import { overrideBindings } from "@/lib/interval-index";

import { FIXTURE } from "./fixture-media";
import { UNVERIFIED_SUFFIX } from "./output";
import {
  frameCount,
  initiallyAnalyzed,
  mockProject,
  mockTracks,
  mockVerifyReport,
} from "./project";

/** What the mock stores per project. */
export interface MockProjectState {
  /** Tracks, edits, bindings and health spans as `get_tracks` returns them. */
  set: TrackSet;
  /** Frame ranges changed by edits since the last verification. */
  unverified: FrameRange[];
  /** Whether an analysis has produced the tracks. */
  analyzed: boolean;
  /** The last verification report, `null` before the first one. */
  report: VerifyReport | null;
  /** The file the last export wrote, if any. */
  lastOutput: string | null;
}

/** An `AppError` without details. */
function failure(kind: AppError["kind"], message: string): AppError {
  return { kind, message, detail: null };
}

/** `VerifySummary::compute`: the counts and result of a report's leaks and regions. */
export function summarize(
  leaks: readonly Leak[],
  perRegion: VerifyReport["per_region"],
): VerifySummary {
  const count = (status: Leak["status"]) =>
    leaks.filter((leak) => leak.kind !== "OutOfScope" && leak.status === status).length;
  const open = count("Open");
  const unverifiable = Object.values(perRegion).reduce(
    (sum, region) => sum + (region?.unverifiable_items ?? 0),
    0,
  );
  return {
    state: open > 0 ? "LeaksRemain" : unverifiable > 0 ? "Unverified" : "Clean",
    open,
    fixed: count("Fixed"),
    accepted: count("Accepted"),
    out_of_scope: leaks.filter((leak) => leak.kind === "OutOfScope").length,
    unverifiable_items: unverifiable,
  };
}

/** Whether two edits are the same (field by field). */
function sameEdit(a: Override, b: Override): boolean {
  return JSON.stringify(a) === JSON.stringify(b);
}

/**
 * Merges ranges into sorted, disjoint ranges (touching ranges join), like
 * the engine keeps `unverified_ranges`.
 */
export function mergeRanges(ranges: readonly FrameRange[]): FrameRange[] {
  const sorted = ranges.toSorted((a, b) => a[0] - b[0]);
  const out: FrameRange[] = [];
  for (const [f0, f1] of sorted) {
    const last = out.at(-1);
    if (last && f0 <= last[1] + 1) last[1] = Math.max(last[1], f1);
    else out.push([f0, f1]);
  }
  return out;
}

/** The frames of every edit that was added, removed or changed. */
export function touchedRanges(
  before: readonly Override[],
  after: readonly Override[],
): FrameRange[] {
  const old = new Map(before.map((edit) => [edit.uid, edit]));
  const now = new Map(after.map((edit) => [edit.uid, edit]));
  const ranges: FrameRange[] = [];
  for (const uid of new Set([...old.keys(), ...now.keys()])) {
    const a = old.get(uid);
    const b = now.get(uid);
    if (a && b && sameEdit(a, b)) continue;
    if (a) ranges.push([a.f0, a.f1]);
    if (b) ranges.push([b.f0, b.f1]);
  }
  return mergeRanges(ranges);
}

/** Why an edit list cannot be stored, or `null` when it can. */
function invalidEdit(overrides: readonly Override[], frames: number): string | null {
  const width = FIXTURE.width;
  const height = FIXTURE.height;
  const inside = ([x0, y0, x1, y1]: RectI) =>
    x0 >= 0 && y0 >= 0 && x1 <= width && y1 <= height && x0 < x1 && y0 < y1;
  const uids = new Set<string>();
  for (const edit of overrides) {
    if (uids.has(edit.uid)) return `edit ${edit.uid} is listed twice`;
    uids.add(edit.uid);
    if (edit.f0 < 0 || edit.f0 > edit.f1 || edit.f1 >= frames) {
      return `edit ${edit.uid} has frames outside the video`;
    }
    const rects =
      edit.op === "Add"
        ? [edit.rect]
        : edit.op === "Adjust"
          ? [edit.anchor, edit.rect]
          : [edit.anchor];
    if (!rects.every(inside)) return `edit ${edit.uid} has a box outside the frame`;
  }
  return null;
}

/** The stored projects of one mock backend, by path. */
export class MockProjects {
  private readonly states = new Map<string, MockProjectState>();

  /** The stored state of `path`, created from the fixture on first use. */
  state(path: string): MockProjectState {
    let state = this.states.get(path);
    if (!state) {
      state = {
        set: mockTracks(path),
        unverified: mergeRanges(mockProject(path).verify.unverified_ranges),
        analyzed: initiallyAnalyzed(path),
        report: mockVerifyReport(path),
        lastOutput: null,
      };
      this.states.set(path, state);
    }
    return state;
  }

  /** `set_overrides`: stores the full list as a new revision. */
  setOverrides(path: string, baseRev: number, overrides: readonly Override[]): OverridesSaved {
    const state = this.state(path);
    if (baseRev !== state.set.revision) {
      throw failure(
        "StaleRevision",
        `The project is at revision ${state.set.revision}, not ${baseRev}.`,
      );
    }
    const problem = invalidEdit(overrides, frameCount(path));
    if (problem !== null) throw failure("InvalidProject", `Invalid edit: ${problem}.`);
    const list = overrides.map((edit) => ({ ...edit }));
    state.unverified = mergeRanges([
      ...state.unverified,
      ...touchedRanges(state.set.overrides, list),
    ]);
    state.set = {
      ...state.set,
      revision: state.set.revision + 1,
      overrides: list,
      bindings: overrideBindings(state.set.tracks, list),
    };
    return { revision: state.set.revision, unverified_ranges: state.unverified };
  }

  /**
   * Simulates another writer (a second window, an imported project): the
   * revision moves on, the edits stay.
   */
  touch(path: string): void {
    const state = this.state(path);
    state.set = { ...state.set, revision: state.set.revision + 1 };
  }

  /**
   * A finished analysis of the `enabled` regions: the fixture's tracks of
   * those regions replace the stored ones, the edits are kept and rebound,
   * the revision moves on. With `verified`, the whole video counts as
   * verified again (no unverified ranges) and the report is renewed.
   */
  analyze(path: string, enabled: ReadonlySet<Region>, verified: boolean): void {
    const state = this.state(path);
    const tracks = mockTracks(path, true).tracks.filter((track) => enabled.has(track.region));
    state.analyzed = true;
    state.set = {
      ...state.set,
      revision: state.set.revision + 1,
      tracks,
      bindings: overrideBindings(tracks, state.set.overrides),
    };
    if (verified) {
      state.unverified = [];
      state.report = mockVerifyReport(path, state.set.revision);
    }
  }

  /**
   * `accept_report`: the open leaks of report `reportId` become accepted
   * and the last output loses its `_UNVERIFIED` suffix; answers the new
   * path.
   */
  accept(path: string, reportId: string, reportName: string | null): string {
    const state = this.state(path);
    const { report, lastOutput } = state;
    if (!report || reportId !== reportName) throw failure("NotFound", "The report does not exist.");
    const at = lastOutput?.lastIndexOf(UNVERIFIED_SUFFIX) ?? -1;
    if (!lastOutput || at === -1) {
      throw failure("InvalidProject", "There is no unverified output to rename.");
    }
    const leaks = report.leaks.map((leak) =>
      leak.status === "Open" ? { ...leak, status: "Accepted" as const } : leak,
    );
    state.report = { ...report, leaks, summary: summarize(leaks, report.per_region) };
    const renamed = lastOutput.slice(0, at) + lastOutput.slice(at + UNVERIFIED_SUFFIX.length);
    state.lastOutput = renamed;
    return renamed;
  }

  /**
   * Simulates a finished re-analysis whose detections sit `dy` pixels
   * lower: the tracks are replaced, the edits are kept and rebound (edits
   * whose anchor no longer matches become unbound), the revision moves on.
   */
  reanalyze(path: string, dy: number): void {
    const state = this.state(path);
    const tracks = state.set.tracks.map((track) => ({
      ...track,
      segments: track.segments.map((segment) => {
        const [x0, y0, x1, y1] = segment.rect;
        return { ...segment, rect: [x0, y0 + dy, x1, y1 + dy] as RectI };
      }),
    }));
    state.set = {
      ...state.set,
      revision: state.set.revision + 1,
      tracks,
      bindings: overrideBindings(tracks, state.set.overrides),
    };
  }
}
