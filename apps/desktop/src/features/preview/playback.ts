/**
 * Frame-accurate playback of the preview video. The store holds what the
 * transport shows (the presented frame, play state, speed); the controller
 * drives the `<video>` element: seeks aim at the middle of a frame's
 * display interval (`FrameClock.seekTime`), and the frame number is only
 * ever taken from the frame the browser actually presented
 * (`requestVideoFrameCallback` → `presented`).
 */
import { create } from "zustand";

import type { FrameClock } from "@/lib/frames";

/** Playback speeds of the speed menu (design 8.1). */
export const RATES = [0.25, 0.5, 1, 1.5, 2] as const;

/** Shuttle multipliers of J / L: each press goes one step further. */
const SHUTTLE = [1, 2, 4] as const;

/** Interval of the reverse shuttle (the video element cannot play backwards). */
const REVERSE_TICK_MS = 100;

/** State of the preview playback. */
export interface PlaybackState {
  /** The presented frame (presentation index). */
  readonly frame: number;
  /** The frame a running seek aims at, until it is presented. */
  readonly target: number | null;
  readonly playing: boolean;
  /** Speed chosen in the speed menu. */
  readonly rate: number;
  /** J / L multiplier while playing: 1, 2 or 4, negative backwards. */
  readonly shuttle: number;
  readonly muted: boolean;
  /** Whether the timecode shows the frame number instead of the time. */
  readonly showFrameNumber: boolean;
}

const INITIAL = {
  frame: 0,
  target: null,
  playing: false,
  shuttle: 1,
} as const satisfies Partial<PlaybackState>;

export const usePlaybackStore = create<PlaybackState>()(() => ({
  ...INITIAL,
  rate: 1,
  muted: true,
  showFrameNumber: false,
}));

/** Back to the first frame, paused (a new project); speed and mute stay. */
export function resetPlayback(): void {
  usePlaybackStore.setState(INITIAL);
}

/** Drives one `<video>` element with the frame clock of its project. */
export class PlaybackController {
  private reverseTimer: ReturnType<typeof setInterval> | null = null;

  constructor(
    readonly video: HTMLVideoElement,
    readonly clock: FrameClock,
  ) {
    const { muted, rate } = usePlaybackStore.getState();
    video.muted = muted;
    video.playbackRate = rate;
  }

  /** Called with every presented frame's media time. */
  presented(mediaTime: number): void {
    const frame = this.clock.frameAt(mediaTime);
    const { target } = usePlaybackStore.getState();
    usePlaybackStore.setState({ frame, target: target === frame ? null : target });
  }

  /** The frame the next relative move starts from. */
  private get current(): number {
    const { frame, target } = usePlaybackStore.getState();
    return target ?? frame;
  }

  /** Shows `frame` (clamped), pausing playback. */
  seek(frame: number): void {
    this.stopReverse();
    const f = this.clock.clamp(frame);
    if (!this.video.paused) this.video.pause();
    usePlaybackStore.setState({ target: f, playing: false, shuttle: 1 });
    this.video.currentTime = this.clock.seekTime(f);
  }

  /** Moves `frames` frames from the current one. */
  step(frames: number): void {
    this.seek(this.current + frames);
  }

  /** Moves by `seconds` (rounded to whole frames). */
  jump(seconds: number): void {
    this.step(Math.round(seconds / this.clock.frameDuration));
  }

  /** Plays forwards at the menu speed times `shuttle`. */
  play(shuttle = 1): void {
    this.stopReverse();
    const { rate } = usePlaybackStore.getState();
    this.video.playbackRate = rate * shuttle;
    if (this.current >= this.clock.count - 1) this.video.currentTime = 0;
    usePlaybackStore.setState({ playing: true, target: null, shuttle });
    void this.video.play().catch(() => usePlaybackStore.setState({ playing: false, shuttle: 1 }));
  }

  pause(): void {
    this.stopReverse();
    this.video.pause();
    usePlaybackStore.setState({ playing: false, shuttle: 1 });
  }

  togglePlay(): void {
    if (usePlaybackStore.getState().playing) this.pause();
    else this.play();
  }

  /** Sets the menu speed; a running playback continues at the new speed. */
  setRate(rate: number): void {
    const { shuttle, playing } = usePlaybackStore.getState();
    usePlaybackStore.setState({ rate });
    if (playing && shuttle > 0) this.video.playbackRate = rate * shuttle;
  }

  /**
   * J / L shuttle: each press in the same direction goes faster (1×, 2×,
   * 4×); the other direction starts at 1× again. Backwards the video is
   * paused and stepped back on a timer.
   */
  shuttle(direction: 1 | -1): void {
    const { playing, shuttle } = usePlaybackStore.getState();
    const same = playing && Math.sign(shuttle) === direction;
    const fastest = SHUTTLE[SHUTTLE.length - 1] ?? 1;
    const speed: number = same ? (SHUTTLE.find((s) => s > Math.abs(shuttle)) ?? fastest) : 1;
    if (direction === 1) {
      this.play(speed);
      return;
    }
    this.stopReverse();
    this.video.pause();
    usePlaybackStore.setState({ playing: true, shuttle: -speed, target: null });
    const { rate } = usePlaybackStore.getState();
    const seconds = (rate * speed * REVERSE_TICK_MS) / 1000;
    const framesPerTick = Math.max(1, Math.round(seconds / this.clock.frameDuration));
    this.reverseTimer = setInterval(() => {
      const next = this.current - framesPerTick;
      if (next <= 0) {
        this.seek(0);
        return;
      }
      usePlaybackStore.setState({ target: next });
      this.video.currentTime = this.clock.seekTime(next);
    }, REVERSE_TICK_MS);
  }

  setMuted(muted: boolean): void {
    this.video.muted = muted;
    usePlaybackStore.setState({ muted });
  }

  /** Called when the video reached its end. */
  ended(): void {
    this.stopReverse();
    usePlaybackStore.setState({ playing: false, shuttle: 1 });
  }

  /** Stops timers; the element itself belongs to the stage. */
  dispose(): void {
    this.stopReverse();
  }

  private stopReverse(): void {
    if (this.reverseTimer !== null) clearInterval(this.reverseTimer);
    this.reverseTimer = null;
  }
}

/** The controller of the shown video, set by the stage while it is mounted. */
export const useControllerStore = create<{ readonly controller: PlaybackController | null }>()(
  () => ({ controller: null }),
);
