import { CloudAlert, CloudCheck, Redo2, SquarePlus, Undo2 } from "lucide-react";
import type { ReactNode } from "react";
import { useStore } from "zustand";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { usePlaybackStore } from "@/features/preview/playback";
import type { LoadedProject } from "@/features/preview/useProject";
import { SHORTCUTS, ariaKeys, shortcutText, type ShortcutId } from "@/lib/shortcuts";
import { redoEdit, undoEdit, useEditsStore } from "@/store/edits";
import { usePreviewStore } from "@/store/preview";

import { toggleManualTool } from "./actions";
import { flushEdits } from "./useProjectEdits";

interface EditButtonProps {
  readonly label: string;
  readonly shortcut: ShortcutId;
  readonly onClick: () => void;
  readonly disabled?: boolean;
  readonly pressed?: boolean;
  readonly children: ReactNode;
}

/** An icon button of the edit controls with its shortcut in the tooltip. */
function EditButton({ label, shortcut, onClick, disabled, pressed, children }: EditButtonProps) {
  const keys = SHORTCUTS[shortcut];
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-md"
          aria-label={label}
          aria-keyshortcuts={ariaKeys(keys)}
          aria-pressed={pressed}
          disabled={disabled}
          className="aria-pressed:bg-selected aria-pressed:text-accent-foreground"
          onClick={onClick}
        >
          {children}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="top">
        {label}
        <Kbd>{shortcutText(keys)}</Kbd>
      </TooltipContent>
    </Tooltip>
  );
}

/**
 * Whether the edits are stored: nothing before the first edit, then
 * "Saving…", "Saved" or "Not saved" with a retry button. A polite live
 * region, so screen readers hear when a save fails.
 */
function SaveState({ edited }: { readonly edited: boolean }) {
  const status = useEditsStore((state) => state.status);
  let content: ReactNode = null;
  if (status === "failed") {
    content = (
      <>
        <CloudAlert aria-hidden className="size-4 text-destructive" />
        <span>Not saved</span>
        <Button variant="ghost" size="sm" onClick={() => void flushEdits()}>
          Retry
        </Button>
      </>
    );
  } else if (status !== "saved") {
    content = (
      <>
        <Spinner label="Saving the edits" className="size-3.5" />
        <span className="hidden @2xl/preview:inline">Saving…</span>
      </>
    );
  } else if (edited) {
    content = (
      <>
        <CloudCheck aria-label="Saved" className="size-4" />
        <span aria-hidden className="hidden @2xl/preview:inline">
          Saved
        </span>
      </>
    );
  }
  return (
    <div
      aria-live="polite"
      data-testid="save-state"
      data-status={status}
      className="flex items-center gap-1 text-ui-sm text-foreground-subtle"
    >
      {content}
    </div>
  );
}

/**
 * The edit controls of the transport (design 8.1): the manual box tool
 * (M), undo and redo of box edits, and whether the edits are saved.
 */
export function EditControls({ data }: { readonly data: LoadedProject }) {
  const toolOn = usePreviewStore((state) => state.tool !== null);
  const canUndo = useStore(useEditsStore.temporal, (state) => state.pastStates.length > 0);
  const canRedo = useStore(useEditsStore.temporal, (state) => state.futureStates.length > 0);

  return (
    <div className="flex items-center gap-1" data-testid="edit-controls">
      <EditButton
        label="Draw a manual box"
        shortcut="manualBox"
        pressed={toolOn}
        onClick={() => {
          const { frame, target } = usePlaybackStore.getState();
          toggleManualTool(target ?? frame, data);
        }}
      >
        <SquarePlus />
      </EditButton>
      <EditButton label="Undo" shortcut="undo" disabled={!canUndo} onClick={undoEdit}>
        <Undo2 />
      </EditButton>
      <EditButton label="Redo" shortcut="redo" disabled={!canRedo} onClick={redoEdit}>
        <Redo2 />
      </EditButton>
      <SaveState edited={canUndo || canRedo} />
    </div>
  );
}
