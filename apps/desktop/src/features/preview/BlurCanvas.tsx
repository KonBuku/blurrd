import { useEffect, useEffectEvent, useRef, useState, type CSSProperties } from "react";

import { fetchExact, type ExactPatch } from "@/ipc/media";
import { prepareBoxes } from "@/lib/blur-geometry";
import { logWarning } from "@/lib/log";
import { usePreviewStore } from "@/store/preview";

import { BlurRenderer, type PreviewMode } from "./gl/blur-renderer";
import { WebGlUnavailableError } from "./gl/gl-kit";
import { usePlaybackStore, type PlaybackController } from "./playback";
import type { LoadedProject } from "./useProject";

export interface BlurCanvasProps {
  readonly video: HTMLVideoElement;
  readonly controller: PlaybackController;
  readonly data: LoadedProject;
  /** Position and size on the stage (same box as the video element). */
  readonly style: CSSProperties;
  /** Called once when WebGL 2 is not available (the video then shows unblurred). */
  readonly onUnavailable: (message: string) => void;
}

/** Exact patches of one frame. */
interface ExactState {
  readonly frame: number;
  readonly patches: readonly ExactPatch[];
}

/** The Exact patches to draw on `frame`: only patches of that very frame. */
function patchesFor(frame: number, on: boolean, exact: ExactState | null): readonly ExactPatch[] {
  return on && exact?.frame === frame ? exact.patches : [];
}

/** Blurs the boxes of `frame` over the uploaded picture and draws it in `mode`. */
function paint(
  renderer: BlurRenderer,
  frame: number,
  data: LoadedProject,
  mode: PreviewMode,
  split: number,
  patches: readonly ExactPatch[],
): void {
  const { width, height } = data.project.media;
  if (!data.params) {
    // Parameters the engine rejects: show the frame unblurred.
    renderer.present("before", 0);
    return;
  }
  const boxes = prepareBoxes(data.index.boxesAt(frame, data.enabled), width, height, data.params);
  renderer.blur(boxes, data.params, data.colour, patches);
  renderer.present(mode, split * width);
}

/**
 * The WebGL canvas over the video element. Every frame the browser
 * presents (`requestVideoFrameCallback`, and `seeked` while paused) is
 * uploaded, its boxes are looked up in the interval index and blurred,
 * and the result is drawn in the current mode. Mode, split and Exact
 * changes redraw the last frame without a new upload. While paused with
 * Exact on, the backend's own blur of the HUD regions replaces the WebGL
 * result there.
 */
export function BlurCanvas({ video, controller, data, style, onUnavailable }: BlurCanvasProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rendererRef = useRef<BlurRenderer | null>(null);
  /** Frame index of the uploaded picture, or -1 before the first one. */
  const uploadedRef = useRef(-1);
  const [uploaded, setUploaded] = useState(-1);
  const [exact, setExact] = useState<ExactState | null>(null);
  const mode = usePreviewStore((state) => state.mode);
  const split = usePreviewStore((state) => state.split);
  const exactOn = usePreviewStore((state) => state.exact);
  const playing = usePlaybackStore((state) => state.playing);
  const { width, height } = data.project.media;

  // A presented frame: upload it and draw it with the current view.
  const show = useEffectEvent((mediaTime: number) => {
    const renderer = rendererRef.current;
    if (!renderer || video.readyState < HTMLMediaElement.HAVE_CURRENT_DATA) return;
    controller.presented(mediaTime);
    const frame = controller.clock.frameAt(mediaTime);
    renderer.upload(video, width, height);
    uploadedRef.current = frame;
    setUploaded(frame);
    paint(renderer, frame, data, mode, split, patchesFor(frame, exactOn, exact));
  });

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return undefined;
    try {
      rendererRef.current = new BlurRenderer(canvas);
    } catch (error) {
      if (!(error instanceof WebGlUnavailableError)) throw error;
      logWarning("preview", error);
      onUnavailable(error.message);
      return undefined;
    }
    let handle = 0;
    const onFrame: VideoFrameRequestCallback = (_now, metadata) => {
      show(metadata.mediaTime);
      handle = video.requestVideoFrameCallback(onFrame);
    };
    handle = video.requestVideoFrameCallback(onFrame);
    // Paused frames: the first decoded frame and finished seeks (the frame
    // callback can run before the element reports the frame as ready).
    const onReady = () => {
      if (video.paused) show(video.currentTime);
    };
    video.addEventListener("loadeddata", onReady);
    video.addEventListener("seeked", onReady);
    onReady();
    return () => {
      video.cancelVideoFrameCallback(handle);
      video.removeEventListener("loadeddata", onReady);
      video.removeEventListener("seeked", onReady);
      rendererRef.current?.dispose();
      rendererRef.current = null;
      uploadedRef.current = -1;
    };
  }, [video, onUnavailable]);

  // Redraw the shown frame when the view, the plan or the patches change.
  useEffect(() => {
    const renderer = rendererRef.current;
    const frame = uploadedRef.current;
    if (renderer && frame >= 0)
      paint(renderer, frame, data, mode, split, patchesFor(frame, exactOn, exact));
  }, [data, mode, split, exactOn, exact]);

  // Exact: fetch the backend patches of a paused frame.
  useEffect(() => {
    if (!exactOn || playing || uploaded < 0) return undefined;
    const abort = new AbortController();
    fetchExact(data.project.id, uploaded, data.tracks.revision, abort.signal)
      .then((result) => setExact(result))
      .catch((error: unknown) => {
        if (!abort.signal.aborted) logWarning(`preview: exact patches of frame ${uploaded}`, error);
      });
    return () => abort.abort();
  }, [exactOn, playing, uploaded, data.project.id, data.tracks.revision]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden
      data-testid="preview-canvas"
      data-frame={uploaded}
      data-exact-frame={exactOn ? exact?.frame : undefined}
      className="absolute"
      style={style}
    />
  );
}
