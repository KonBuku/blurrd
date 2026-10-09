/**
 * Frame-accurate time ↔ frame mapping from the presentation timestamps of
 * the decoded video (`pts.bin`, plan 6.2). The browser reports the media
 * time of the presented frame (`requestVideoFrameCallback`); the frame
 * index is the last frame whose timestamp is not after it. Seeks aim at
 * the middle of a frame's display interval, so rounding in the browser or
 * in a container's time base never lands on the neighbour.
 */
import type { Rational } from "@/bindings/Rational";

/**
 * Slack when comparing media times: browsers report frame start times
 * with float noise of a few microseconds at most.
 */
const EPSILON = 1e-5;

/** Presentation times of every frame, in seconds. */
export class FrameClock {
  /** Start time of each frame in seconds, ascending. */
  readonly times: Float64Array;
  /** Nominal frame duration in seconds (from the average rate). */
  readonly frameDuration: number;

  constructor(pts: readonly number[], timeBase: Rational, fps: Rational) {
    const [num, den] = timeBase;
    const first = pts[0] ?? 0;
    // Times are relative to the first frame, like the browser's media time.
    this.times = Float64Array.from(pts, (p) => ((p - first) * num) / den);
    this.frameDuration = fps[0] > 0 ? fps[1] / fps[0] : 1 / 60;
  }

  /** Number of frames. */
  get count(): number {
    return this.times.length;
  }

  /** Duration from the first frame to the end of the last one, in seconds. */
  get duration(): number {
    const last = this.times.at(-1);
    return last === undefined ? 0 : last + this.frameDuration;
  }

  /** The frame shown at media time `seconds` (clamped to the video). */
  frameAt(seconds: number): number {
    const { times } = this;
    let lo = 0;
    let hi = times.length - 1;
    if (hi < 0) return 0;
    while (lo < hi) {
      const mid = (lo + hi + 1) >> 1;
      if ((times[mid] ?? 0) <= seconds + EPSILON) lo = mid;
      else hi = mid - 1;
    }
    return lo;
  }

  /** Start time of frame `frame` in seconds. */
  startOf(frame: number): number {
    return this.times[this.clamp(frame)] ?? 0;
  }

  /** The media time to seek to so that `frame` is shown: the middle of its interval. */
  seekTime(frame: number): number {
    const f = this.clamp(frame);
    const start = this.times[f] ?? 0;
    const next = this.times[f + 1] ?? start + this.frameDuration;
    return (start + next) / 2;
  }

  /** `frame` limited to the video's frames. */
  clamp(frame: number): number {
    return Math.min(Math.max(Math.round(frame), 0), Math.max(this.count - 1, 0));
  }
}

/** `v` with at least two digits. */
function two(v: number): string {
  return String(v).padStart(2, "0");
}

/**
 * A media time as `hh:mm:ss.cc` (hundredths), the transport's timecode
 * (design 8.1: "00:01:12.34").
 */
export function formatTimecode(seconds: number): string {
  const total = Math.max(0, Math.floor(seconds * 100 + 1e-6));
  const cc = total % 100;
  const s = Math.floor(total / 100) % 60;
  const m = Math.floor(total / 6000) % 60;
  const h = Math.floor(total / 360000);
  return `${two(h)}:${two(m)}:${two(s)}.${two(cc)}`;
}
