import brandIconLight from "@brand/blurrd-icon-light.svg";
import brandIcon from "@brand/blurrd-icon.svg";
import { PanelRightClose, PanelRightOpen, TriangleAlert } from "lucide-react";
import { useCallback, useState, type ReactNode } from "react";

import { Button } from "@/components/ui/button";
import { Kbd } from "@/components/ui/kbd";
import { Spinner } from "@/components/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { EmptyState } from "@/features/empty/EmptyState";
import { INSPECTOR_ID } from "@/features/inspector/Inspector";
import { Frame, FrameHeader, FrameTitle } from "@/features/shell/Frame";
import { baseName, previewSummary } from "@/lib/media";
import { SHORTCUTS } from "@/lib/shortcuts";
import { usePreviewStore } from "@/store/preview";
import { useQueueStore } from "@/store/queue";

import { useVerifyReport } from "@/features/timeline/useTimelineData";
import { Timeline } from "@/features/timeline/Timeline";

import { BoxOverlay } from "./BoxOverlay";
import { PreviewToolbar } from "./PreviewToolbar";
import { Transport } from "./Transport";
import { useProject } from "./useProject";
import { VideoStage, type StageStatus } from "./VideoStage";

/** Id of the inspector toggle in the preview header. */
export const INSPECTOR_TOGGLE_ID = "inspector-toggle";

export interface PreviewFrameProps {
  /** Whether the inspector (frame or sheet) is currently shown. */
  readonly inspectorShown: boolean;
  /** Shows or hides the inspector. */
  readonly onToggleInspector: () => void;
}

/**
 * The queue item the preview shows: the focused item when it is selected,
 * otherwise the first selected one; only once its media facts are known.
 */
function useShownPath(): string | null {
  return useQueueStore((state) => {
    const { focused, selected, items } = state;
    const id = focused !== null && selected.has(focused) ? focused : selected.values().next().value;
    const item = items.find((candidate) => candidate.id === id);
    return item?.media ? item.path : null;
  });
}

/**
 * The preview frame: a header with the file, the preview controls and
 * the inspector toggle, and the stage below it. With an empty queue the
 * stage shows the empty state (design 8.2), with nothing selected the
 * welcome message.
 */
export function PreviewFrame({ inspectorShown, onToggleInspector }: PreviewFrameProps) {
  const label = inspectorShown ? "Hide inspector" : "Show inspector";
  const { keys } = SHORTCUTS.toggleInspector;
  const queueEmpty = useQueueStore((state) => state.items.length === 0);
  const path = useShownPath();

  const toggle = (
    <Tooltip>
      <TooltipTrigger asChild>
        <Button
          variant="ghost"
          size="icon-md"
          id={INSPECTOR_TOGGLE_ID}
          aria-label={label}
          aria-controls={INSPECTOR_ID}
          aria-expanded={inspectorShown}
          className="text-foreground-subtle hover:text-foreground"
          onClick={onToggleInspector}
        >
          {inspectorShown ? <PanelRightClose /> : <PanelRightOpen />}
        </Button>
      </TooltipTrigger>
      <TooltipContent side="bottom">
        {label}
        <Kbd>{keys.join("+")}</Kbd>
      </TooltipContent>
    </Tooltip>
  );

  if (path !== null) return <ProjectPreview key={path} path={path} toggle={toggle} />;
  return (
    <Frame aria-labelledby="preview-title" className="flex-1">
      <FrameHeader>
        <FrameTitle id="preview-title">Preview</FrameTitle>
        <div className="flex-1" />
        {toggle}
      </FrameHeader>
      {queueEmpty ? (
        <EmptyState />
      ) : (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-3 p-6 text-center">
          <img src={brandIcon} alt="" className="size-16 rounded-2xl dark:hidden" />
          <img src={brandIconLight} alt="" className="hidden size-16 rounded-2xl dark:block" />
          <div className="flex flex-col gap-1">
            <h1 className="text-ui-lg font-medium">Blur player names in your clips</h1>
            <p className="text-ui-base text-foreground-subtle">
              Select a recording in the queue to preview it. Everything runs on this PC.
            </p>
          </div>
        </div>
      )}
    </Frame>
  );
}

interface ProjectPreviewProps {
  readonly path: string;
  /** The inspector toggle of the header. */
  readonly toggle: ReactNode;
}

/** The preview of one file: header with its facts and controls, then the stage. */
function ProjectPreview({ path, toggle }: ProjectPreviewProps) {
  const state = useProject(path);
  const report = useVerifyReport(state.status === "ready" ? state.data.project.id : "");
  const showBoxes = usePreviewStore((store) => store.showBoxes);
  const [status, setStatus] = useState<StageStatus>({ proxy: false, webglProblem: null });
  const onStatus = useCallback((next: StageStatus) => setStatus(next), []);
  const project = state.status === "ready" ? state.data.project : null;

  return (
    <Frame aria-labelledby="preview-title" className="@container/preview flex-1">
      <FrameHeader>
        <div className="flex min-w-0 flex-1 items-baseline gap-2">
          <FrameTitle id="preview-title" level={1} className="shrink">
            {baseName(path)}
          </FrameTitle>
          {project && (
            <span className="hidden min-w-0 truncate font-mono text-ui-sm text-foreground-subtle tabular-nums @3xl/preview:inline">
              {previewSummary(project.media)}
            </span>
          )}
        </div>
        {state.status === "ready" && <PreviewToolbar status={status} />}
        {toggle}
      </FrameHeader>
      {state.status === "loading" && (
        <div className="flex flex-1 items-center justify-center bg-video-stage text-white">
          <Spinner label="Opening the recording" />
        </div>
      )}
      {state.status === "error" && (
        <div className="flex flex-1 items-center justify-center bg-video-stage p-8">
          <div role="alert" className="flex max-w-md items-start gap-2 text-ui-sm text-white">
            <TriangleAlert className="mt-0.5 size-4 shrink-0 text-warning" />
            <p>The recording could not be opened: {state.error.message}</p>
          </div>
        </div>
      )}
      {state.status === "ready" && (
        <>
          <VideoStage data={state.data} onStatus={onStatus}>
            {(box) =>
              showBoxes && <BoxOverlay data={state.data} report={report.data ?? null} box={box} />
            }
          </VideoStage>
          <Transport data={state.data} />
          <Timeline data={state.data} report={report.data ?? null} />
        </>
      )}
    </Frame>
  );
}
