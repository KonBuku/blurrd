import { Plus, Trash2 } from "lucide-react";

import type { Override } from "@/bindings/Override";
import type { ProjectSummary } from "@/bindings/ProjectSummary";
import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { RegionIcon } from "@/components/ui/region-chip";
import { editKey, removeEditByUid, startManualTool } from "@/features/editing/actions";
import { useControllerStore, usePlaybackStore } from "@/features/preview/playback";
import { formatTimecode } from "@/lib/frames";
import { cn } from "@/lib/utils";
import { useEditsStore } from "@/store/edits";
import { usePreviewStore } from "@/store/preview";

import { InspectorSection } from "./parts";

export interface ManualBoxesProps {
  readonly project: ProjectSummary;
}

/** A manual box of the edit list. */
type ManualBox = Extract<Override, { op: "Add" }>;

/**
 * Inspector › Blur › Manual boxes (design 8.1): the boxes the user drew on
 * the shown file with their time ranges; a row jumps the preview to the
 * box and selects it, the trash button removes it (one undo step), and
 * "Add manual box" starts the tool around the current frame.
 */
export function ManualBoxes({ project }: ManualBoxesProps) {
  const boxes = useEditsStore((state) => (state.projectId === project.id ? state.overrides : null));
  const selected = usePreviewStore((state) => state.selected);
  const controller = useControllerStore((state) => state.controller);
  const [num, den] = project.media.fps;
  const fps = num / den;
  const manual = (boxes ?? []).filter(
    (edit): edit is ManualBox => edit.op === "Add" && edit.region === "Manual",
  );
  const time = (frame: number) => formatTimecode(frame / fps);

  return (
    <InspectorSection
      title="Manual boxes"
      aside={
        <span className="text-ui-xs text-foreground-subtle tabular-nums">{manual.length}</span>
      }
    >
      {manual.length > 0 && (
        <ul aria-label="Manual boxes" className="flex flex-col">
          {manual.map((box) => (
            <li
              key={box.uid}
              className={cn(
                "flex items-center gap-1 border-b border-border py-1 pr-1 pl-3",
                selected === editKey(box.uid) && "bg-selected",
              )}
            >
              <button
                type="button"
                disabled={!controller}
                onClick={() => {
                  controller?.seek(box.f0);
                  usePreviewStore.getState().select(editKey(box.uid));
                }}
                className="flex min-w-0 flex-1 cursor-default items-center gap-2 rounded-md py-1 text-left hover:text-foreground disabled:opacity-50"
              >
                <RegionIcon region="Manual" />
                <span className="truncate font-mono text-ui-sm tabular-nums">
                  {time(box.f0)}–{time(box.f1)}
                </span>
                <span className="sr-only">
                  , frames {box.f0} to {box.f1}
                </span>
              </button>
              <Button
                variant="ghost"
                size="icon-sm"
                aria-label={`Remove manual box ${time(box.f0)}–${time(box.f1)}`}
                onClick={() => removeEditByUid(box.uid)}
              >
                <Trash2 strokeWidth={1.5} />
              </Button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex items-center justify-between gap-2 px-3 py-2">
        <Button
          variant="ghost"
          size="sm"
          disabled={!controller || !boxes}
          onClick={() =>
            startManualTool(
              usePlaybackStore.getState().frame,
              fps,
              project.media.frame_count ?? Math.round(project.media.duration_s * fps),
            )
          }
        >
          <Plus strokeWidth={1.5} />
          Add manual box
        </Button>
        <Kbd>M</Kbd>
      </div>
    </InspectorSection>
  );
}
