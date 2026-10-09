import { useEffect, useEffectEvent } from "react";

import {
  deleteItem,
  nudgeItem,
  resolveSelection,
  setManualRange,
  toggleManualTool,
} from "@/features/editing/actions";
import { useVerifyReport } from "@/features/timeline/useTimelineData";
import { MOVE, withIn, withOut, type EdgeMask } from "@/lib/edits";
import { matchShortcut, targetOwnsKey } from "@/lib/shortcuts";
import { stepZoom } from "@/lib/stage";
import { redoEdit, undoEdit } from "@/store/edits";
import { usePreviewStore } from "@/store/preview";
import { useUiStore } from "@/store/ui";

import { nextMark } from "./overlay-model";

/** Pixel steps of the arrow keys. */
const ARROWS: Readonly<Record<string, readonly [number, number]>> = {
  ArrowLeft: [-1, 0],
  ArrowRight: [1, 0],
  ArrowUp: [0, -1],
  ArrowDown: [0, 1],
};

/** Resizing from the keyboard moves the right and bottom edges. */
const GROW: EdgeMask = { left: false, top: false, right: true, bottom: true };
import { usePlaybackStore, type PlaybackController } from "./playback";
import type { LoadedProject } from "./useProject";

/**
 * The preview shortcuts (design 8.8): transport (Space / K, J / L,
 * arrows, Shift+arrows, Home / End), Before / After / Split (B / A / S),
 * the box overlay (O), next / previous detection or leak (N / Shift+N),
 * zoom (Ctrl+= / Ctrl+- / Ctrl+0) and editing: the manual box tool (M)
 * with its in and out points (I / O), Delete, Alt+arrows to move and
 * Alt+Shift+arrows to resize the selected box, undo and redo (Ctrl+Z,
 * Ctrl+Shift+Z / Ctrl+Y) and Esc. Active while the workspace shows
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
    const selection = resolveSelection(preview.selected, current, data);
    const manual = selection?.edit.kind === "manual" ? selection.edit : null;
    // O ends the manual box while there is one to end (see SHORTCUTS).
    const setsOut = match.id === "toggleBoxes" && (preview.tool !== null || manual !== null);
    const id = setsOut ? "setOut" : match.id;
    switch (id) {
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
      case "manualBox":
        toggleManualTool(current, data);
        break;
      case "setIn":
      case "setOut": {
        const set = id === "setIn" ? withIn : withOut;
        // The tool's range first: while drawing, I and O set the next box.
        if (preview.tool) preview.setTool(set(preview.tool.range, current));
        else if (manual) setManualRange(manual.uid, set(manual.range, current));
        else return;
        break;
      }
      case "deleteBox":
        if (!selection) return;
        deleteItem(selection.edit);
        break;
      case "nudgeBox":
      case "growBox": {
        if (!selection || selection.edit.kind === "edit") return;
        const [dx, dy] = ARROWS[event.key] ?? [0, 0];
        const edges = id === "nudgeBox" ? MOVE : GROW;
        nudgeItem(selection.edit, selection.rect, edges, dx, dy, data);
        break;
      }
      case "undo":
        undoEdit();
        break;
      case "redo":
        redoEdit();
        break;
      case "cancelEdit":
        if (preview.tool) preview.endTool();
        else if (preview.selected !== null) preview.select(null);
        else return;
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
