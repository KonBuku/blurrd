import { TriangleAlert } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { useElementSize } from "@/hooks/useElementSize";
import { stageBox, type StageBox } from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";

import { BlurCanvas } from "./BlurCanvas";
import {
  PlaybackController,
  resetPlayback,
  useControllerStore,
  usePlaybackStore,
} from "./playback";
import { fallbackSource, initialSource, problemText, type PreviewSource } from "./source";
import { SplitDivider } from "./SplitDivider";
import type { LoadedProject } from "./useProject";
import { usePan } from "./usePan";
import { usePreviewShortcuts } from "./usePreviewShortcuts";

export interface VideoStageProps {
  readonly data: LoadedProject;
  /** Layers drawn over the frame (the box overlay), given the frame's box. */
  readonly children?: (box: StageBox) => ReactNode;
  /** Reports which stream plays and whether the preview can blur. */
  readonly onStatus: (status: StageStatus) => void;
}

/** What the stage reports to the toolbar. */
export interface StageStatus {
  /** Whether the proxy plays instead of the source. */
  readonly proxy: boolean;
  /** Set when the preview cannot blur (WebGL 2 missing). */
  readonly webglProblem: string | null;
}

/** Asks the browser whether it can decode a MIME type. */
function canPlay(type: string): CanPlayTypeResult {
  return document.createElement("video").canPlayType(type);
}

/** Shows the frame the playback store points at (after a new stream loaded). */
function restoreFrame(controller: PlaybackController): void {
  const { frame, target } = usePlaybackStore.getState();
  controller.seek(target ?? frame);
}

/**
 * The video stage (design 8.1): the `<video>` element with the WebGL blur
 * canvas on top, letterboxed on the stage colour, zoomable (Ctrl+wheel,
 * zoom menu) and pannable (drag or wheel when zoomed in). The source plays
 * when the browser can decode it; otherwise the proxy does, and a load or
 * decode error switches to the proxy once. Mounted once per project.
 */
export function VideoStage({ data, children, onStatus }: VideoStageProps) {
  const { project, clock } = data;
  const stageRef = useRef<HTMLDivElement>(null);
  const [stageElement, setStageElement] = useState<HTMLDivElement | null>(null);
  const stage = useElementSize(stageRef);
  const [video, setVideo] = useState<HTMLVideoElement | null>(null);
  const [source, setSource] = useState<PreviewSource>(() => initialSource(project, canPlay));
  const [failed, setFailed] = useState(false);
  const [webglProblem, setWebglProblem] = useState<string | null>(null);
  const zoom = usePreviewStore((state) => state.zoom);
  const center = usePreviewStore((state) => state.center);
  const mode = usePreviewStore((state) => state.mode);
  const media = { width: project.media.width, height: project.media.height };
  const box = stageBox(stage, media, zoom, center);

  // One controller per element; a new stream keeps the frame.
  const controller = useMemo(
    () => (video ? new PlaybackController(video, clock) : null),
    [video, clock],
  );

  useEffect(
    () => onStatus({ proxy: source.proxy, webglProblem }),
    [source.proxy, webglProblem, onStatus],
  );

  // The stage mounts per project, which starts at its first frame with
  // nothing selected and no tool active.
  useEffect(() => {
    resetPlayback();
    usePreviewStore.getState().resetEditing();
  }, []);

  useEffect(() => {
    if (!controller) return undefined;
    // The element may have loaded before the controller existed.
    if (controller.video.readyState >= HTMLMediaElement.HAVE_METADATA) restoreFrame(controller);
    useControllerStore.setState({ controller });
    return () => {
      controller.dispose();
      useControllerStore.setState({ controller: null });
    };
  }, [controller]);

  usePan(stageElement, box, stage, media);
  usePreviewShortcuts(data, controller, box.scale);
  // Nearest-neighbour from 200 % on, so single pixels stay square.
  const style = {
    left: box.left,
    top: box.top,
    width: box.width,
    height: box.height,
    imageRendering: box.scale >= 2 ? "pixelated" : "auto",
  } as const;

  return (
    <div
      ref={(element) => {
        stageRef.current = element;
        setStageElement(element);
      }}
      className="relative min-h-0 flex-1 touch-none overflow-hidden bg-video-stage select-none"
      data-testid="video-stage"
    >
      {/* oxlint-disable-next-line jsx-a11y/media-has-caption -- game footage has no captions; the preview is muted by default */}
      <video
        ref={setVideo}
        key={source.url}
        src={source.url}
        preload="auto"
        playsInline
        disablePictureInPicture
        className="absolute"
        style={style}
        onLoadedMetadata={() => {
          if (controller) restoreFrame(controller);
        }}
        onError={() => {
          const next = fallbackSource(project, source);
          if (next) setSource(next);
          else setFailed(true);
        }}
        onEnded={() => controller?.ended()}
      />
      {video && controller && !failed && webglProblem === null && (
        <BlurCanvas
          video={video}
          controller={controller}
          data={data}
          style={style}
          onUnavailable={setWebglProblem}
        />
      )}
      {!failed && children?.(box)}
      {!failed && mode === "split" && <SplitDivider box={box} />}
      {failed && (
        <div className="absolute inset-0 flex items-center justify-center p-8">
          <div
            role="alert"
            className="flex max-w-md items-start gap-2 rounded-lg bg-black/60 p-3 text-ui-sm text-white"
          >
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
            <p>{problemText(project)}</p>
          </div>
        </div>
      )}
    </div>
  );
}
