import { FolderOpen } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Card } from "@/components/ui/card";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { baseName } from "@/lib/media";
import { cn } from "@/lib/utils";

import { showInFolder } from "./batch-end";
import { useJobsStore } from "./jobs";

/**
 * The "Recent exports" card of the empty state (design 8.2): the files
 * exported in this session, newest first, each with its result and a
 * button that shows it in Explorer. Kept in memory only, so blurrd keeps
 * no list of file names on disk beyond the queue the user chose to
 * restore. Hidden until something was exported.
 */
export function RecentExports({ className }: { readonly className?: string }) {
  const recent = useJobsStore((state) => state.recent);
  if (recent.length === 0) return null;

  return (
    <Card aria-label="Recent exports" className={cn("gap-2 px-3 py-3", className)}>
      <h2 className="text-ui-base font-medium">Recent exports</h2>
      <ul className="flex flex-col gap-1">
        {recent.map((entry) => {
          const name = baseName(entry.path);
          return (
            <li key={entry.path} className="flex items-center gap-2">
              <div className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-ui-sm">{name}</span>
                {entry.text !== "" && (
                  <span className="truncate text-ui-xs text-foreground-subtle">{entry.text}</span>
                )}
              </div>
              <Tooltip>
                <TooltipTrigger asChild>
                  <Button
                    variant="ghost"
                    size="icon-sm"
                    aria-label={`Show ${name} in folder`}
                    className="text-foreground-subtle hover:text-foreground"
                    onClick={() => void showInFolder(entry.path)}
                  >
                    <FolderOpen />
                  </Button>
                </TooltipTrigger>
                <TooltipContent side="top">Show in folder</TooltipContent>
              </Tooltip>
            </li>
          );
        })}
      </ul>
    </Card>
  );
}
