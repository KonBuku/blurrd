import {
  Columns2,
  Eye,
  EyeOff,
  Maximize,
  SkipBack,
  SkipForward,
  SquareDashed,
  SquareDashedMousePointer,
} from "lucide-react";
import { useMemo } from "react";

import { toggleManualTool } from "@/features/editing/actions";
import { useCommandSource, type PaletteCommand } from "@/features/command/registry";
import { useVerifyReport } from "@/features/timeline/useTimelineData";
import { usePreviewStore } from "@/store/preview";

import type { PreviewMode } from "./gl/blur-renderer";
import { nextMark } from "./overlay-model";
import { usePlaybackStore, type PlaybackController } from "./playback";
import type { LoadedProject } from "./useProject";

/** The Before / After / Split commands, with their shortcut ids. */
const MODES: readonly {
  mode: PreviewMode;
  label: string;
  shortcut: "showBefore" | "showAfter" | "showSplit";
  icon: PaletteCommand["icon"];
}[] = [
  {
    mode: "before",
    label: "Show the recording before blurring",
    shortcut: "showBefore",
    icon: <EyeOff aria-hidden />,
  },
  {
    mode: "after",
    label: "Show the blurred result",
    shortcut: "showAfter",
    icon: <Eye aria-hidden />,
  },
  {
    mode: "split",
    label: "Show before and after side by side",
    shortcut: "showSplit",
    icon: <Columns2 aria-hidden />,
  },
];

/** The frame the preview shows or is seeking to. */
function currentFrame(): number {
  const { frame, target } = usePlaybackStore.getState();
  return target ?? frame;
}

/**
 * Offers the preview's actions in the command palette while a recording
 * is on the stage: Before / After / Split, the box overlay, the next and
 * previous detection or leak, Fit and the manual box tool. They do what
 * their shortcuts do (see `usePreviewShortcuts`).
 */
export function usePreviewCommands(data: LoadedProject, controller: PlaybackController | null) {
  const mode = usePreviewStore((state) => state.mode);
  const showBoxes = usePreviewStore((state) => state.showBoxes);
  const drawing = usePreviewStore((state) => state.tool !== null);
  const report = useVerifyReport(data.project.id).data ?? null;

  const commands = useMemo((): readonly PaletteCommand[] => {
    if (!controller) return [];
    const preview = () => usePreviewStore.getState();
    const jump = (direction: 1 | -1) => {
      const mark = nextMark(currentFrame(), direction, data.index, report);
      if (mark !== null) controller.seek(mark);
    };
    return [
      ...MODES.map((item): PaletteCommand => ({
        id: `preview.${item.mode}`,
        group: "Preview",
        label: item.label,
        keywords: [item.mode, "compare"],
        icon: item.icon,
        shortcut: item.shortcut,
        hint: item.mode === mode ? "Current" : undefined,
        run: () => preview().setMode(item.mode),
      })),
      {
        id: "preview.toggle-boxes",
        group: "Preview",
        label: showBoxes ? "Hide the box overlay" : "Show the box overlay",
        keywords: ["detections", "boxes"],
        icon: <SquareDashed aria-hidden />,
        shortcut: "toggleBoxes",
        run: () => preview().toggleBoxes(),
      },
      {
        id: "preview.next-mark",
        group: "Preview",
        label: "Go to the next detection or leak",
        keywords: ["jump", "leak", "detection"],
        icon: <SkipForward aria-hidden />,
        shortcut: "nextMark",
        run: () => jump(1),
      },
      {
        id: "preview.previous-mark",
        group: "Preview",
        label: "Go to the previous detection or leak",
        keywords: ["jump", "leak", "detection"],
        icon: <SkipBack aria-hidden />,
        shortcut: "previousMark",
        run: () => jump(-1),
      },
      {
        id: "preview.fit",
        group: "Preview",
        label: "Fit the preview to the stage",
        keywords: ["zoom", "reset"],
        icon: <Maximize aria-hidden />,
        shortcut: "zoomFit",
        run: () => preview().setZoom("fit"),
      },
      {
        id: "preview.manual-box",
        group: "Preview",
        label: drawing ? "Stop drawing the manual box" : "Draw a manual box",
        keywords: ["edit", "add box", "tool"],
        icon: <SquareDashedMousePointer aria-hidden />,
        shortcut: "manualBox",
        run: () => toggleManualTool(currentFrame(), data),
      },
    ];
  }, [controller, data, report, mode, showBoxes, drawing]);

  useCommandSource("preview", commands);
}
