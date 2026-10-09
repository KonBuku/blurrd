import { useEffect, useEffectEvent } from "react";

import { useVerifyReport } from "@/features/timeline/useTimelineData";
import { matchShortcut, targetOwnsKey } from "@/lib/shortcuts";
import { stepZoom } from "@/lib/stage";
import { usePreviewStore } from "@/store/preview";
import { useUiStore } from "@/store/ui";

import { nextMark } from "./overlay-model";
import { usePlaybackStore, type PlaybackController } from "./playback";
import type { LoadedProject } from "./useProject";

/**
 * The preview shortcuts (design 8.8): transport (Space / K, J / L,
 * arrows, Shift+arrows, Home / End), Before / After / Split (B / A / S),
 * the box overlay (O), next / previous detection or leak (N / Shift+N)
 * and zoom (Ctrl+= / Ctrl+- / Ctrl+0). Active while the workspace shows
 * the stage; keys that the focused control uses itself, and keys another
 * handler already consumed, are left alone.
 */
export function usePreviewShortcuts(
  data: LoadedProject,
  controller: PlaybackController | null,
  /** The stage's current scale, where zoom steps start from. */
  scale: number,
): void {
  const report = useVerifyReport(data.project.id).data ?? null;

  const onKeyDown = useEffectEvent((event: KeyboardEvent) => {
    if (event.defaultPrevented || !controller || targetOwnsKey(event)) return;
    if (useUiStore.getState().view !== "workspace") return;
    const match = matchShortcut(event, "preview");
    if (!match) return;
    const preview = usePreviewStore.getState();
    const { frame, target } = usePlaybackStore.getState();
    const current = target ?? frame;
    switch (match.id) {
      case "playPause":
        controller.togglePlay();
        break;
      case "shuttleBack":
        controller.shuttle(-1);
        break;
      case "shuttleForward":
        controller.shuttle(1);
        break;
      case "previousFrame":
        controller.step(-1);
        break;
      case "nextFrame":
        controller.step(1);
        break;
      case "backSecond":
        controller.jump(-1);
        break;
      case "forwardSecond":
        controller.jump(1);
        break;
      case "goToStart":
        controller.seek(0);
        break;
      case "goToEnd":
        controller.seek(data.clock.count - 1);
        break;
      case "showBefore":
        preview.setMode("before");
        break;
      case "showAfter":
        preview.setMode("after");
        break;
      case "showSplit":
        preview.setMode("split");
        break;
      case "toggleBoxes":
        preview.toggleBoxes();
        break;
      case "nextMark":
      case "previousMark": {
        const mark = nextMark(current, match.id === "nextMark" ? 1 : -1, data.index, report);
        if (mark !== null) controller.seek(mark);
        break;
      }
      case "zoomIn":
        preview.setZoom(stepZoom(scale, 1));
        break;
      case "zoomOut":
        preview.setZoom(stepZoom(scale, -1));
        break;
      case "zoomFit":
        preview.setZoom("fit");
        break;
      default:
        return;
    }
    event.preventDefault();
  });

  useEffect(() => {
    const listener = (event: KeyboardEvent) => onKeyDown(event);
    window.addEventListener("keydown", listener);
    return () => window.removeEventListener("keydown", listener);
  }, []);
}
