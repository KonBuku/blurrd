import brandIconLight from "@brand/blurrd-icon-light.svg";
import brandIcon from "@brand/blurrd-icon.svg";
import { FolderPlus, Plus, X } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { browseFiles, browseFolder } from "@/features/import/import";
import { GpuStatusCard } from "@/features/gpu/GpuStatusCard";
import { RecentExports } from "@/features/jobs/RecentExports";
import { SHORTCUTS } from "@/lib/shortcuts";

/** Local storage key that hides the first-run checklist for good. */
export const CHECKLIST_KEY = "blurrd.checklist";

/** Steps of the first-run checklist (design 8.2). */
const STEPS = ["Add clips", "Choose what to blur", "Export"] as const;

/** Whether the checklist was dismissed before; storage errors count as not dismissed. */
function checklistDismissed(): boolean {
  try {
    return localStorage.getItem(CHECKLIST_KEY) === "dismissed";
  } catch {
    return false;
  }
}

/** Remembers that the checklist was dismissed (a per-install convenience). */
function dismissChecklist(): void {
  try {
    localStorage.setItem(CHECKLIST_KEY, "dismissed");
  } catch {
    // Storage is unavailable; the checklist returns on the next start.
  }
}

/**
 * The preview frame while the queue is empty (design 8.2): brand mark,
 * what blurrd does, the two ways to add recordings, the accepted formats,
 * the GPU card, the files exported in this session and, until
 * dismissed, a three-step checklist. The whole window is a drop target,
 * so the text invites dropping too.
 */
export function EmptyState() {
  const [showChecklist, setShowChecklist] = useState(() => !checklistDismissed());

  return (
    <div className="flex min-h-0 flex-1 flex-col items-center justify-center gap-6 overflow-y-auto p-6 text-center">
      <img src={brandIcon} alt="" className="size-16 rounded-2xl dark:hidden" />
      <img src={brandIconLight} alt="" className="hidden size-16 rounded-2xl dark:block" />
      <div className="flex flex-col gap-1">
        <h1 className="text-ui-lg font-medium">Blur player names in your clips</h1>
        <p className="text-ui-base text-foreground-subtle">
          Drop recordings here, or browse. Nothing leaves your PC.
        </p>
      </div>
      <div className="flex flex-col items-center gap-2">
        <div className="flex flex-wrap justify-center gap-2">
          <Button
            size="lg"
            aria-keyshortcuts={SHORTCUTS.addFiles.keys.join("+")}
            onClick={() => void browseFiles()}
          >
            <Plus />
            Browse files…
          </Button>
          <Button
            size="lg"
            variant="outline"
            aria-keyshortcuts={SHORTCUTS.addFolder.keys.join("+")}
            onClick={() => void browseFolder()}
          >
            <FolderPlus />
            Add folder…
          </Button>
        </div>
        <p className="text-ui-sm text-foreground-subtle">MP4 · MOV · MKV — H.264 / HEVC / AV1</p>
      </div>
      <div className="flex w-full max-w-xl flex-wrap justify-center gap-3 text-left">
        <GpuStatusCard variant="full" className="min-w-64 flex-1" />
        <RecentExports className="min-w-64 flex-1" />
        {showChecklist && (
          <Card aria-label="Getting started" className="relative min-w-64 flex-1 gap-2 px-3 py-3">
            <h2 className="pr-6 text-ui-base font-medium">Getting started</h2>
            <ol className="flex flex-col gap-1 text-ui-sm text-foreground-subtle">
              {STEPS.map((step, index) => (
                <li key={step} className="flex items-center gap-2">
                  <span className="grid size-4 shrink-0 place-items-center rounded-full bg-surface text-ui-xs font-medium text-foreground tabular-nums">
                    {index + 1}
                  </span>
                  {step}
                </li>
              ))}
            </ol>
            <Button
              variant="ghost"
              size="icon-sm"
              aria-label="Dismiss getting started"
              className="absolute top-2 right-2 text-foreground-subtle hover:text-foreground"
              onClick={() => {
                dismissChecklist();
                setShowChecklist(false);
              }}
            >
              <X />
            </Button>
          </Card>
        )}
      </div>
    </div>
  );
}
