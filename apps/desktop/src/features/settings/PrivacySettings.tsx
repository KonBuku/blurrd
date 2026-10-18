import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, Trash2 } from "lucide-react";
import { useId, useState } from "react";
import { toast } from "sonner";

import type { LogLevel } from "@/bindings/LogLevel";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { toAppError } from "@/ipc/errors";
import { clearCache, getCacheInfo, openLogDir } from "@/ipc/settings";
import { formatBytes } from "@/lib/media";
import { useQueueStore } from "@/store/queue";
import { useSettingsStore } from "@/store/settings";

import { ReadOnlyNotice, SettingsGroup, SettingsPage, SettingsRow } from "./SettingsRow";

const LOG_LEVELS: readonly { value: LogLevel; label: string }[] = [
  { value: "Error", label: "Errors only" },
  { value: "Warn", label: "Warnings" },
  { value: "Info", label: "Normal (default)" },
  { value: "Debug", label: "Detailed" },
  { value: "Trace", label: "Everything" },
];

/** Query key of the cache figures. */
const CACHE_KEY = ["cache-info"] as const;

/** Opens the log folder in Explorer; a failure is shown. */
async function openLogs() {
  try {
    await openLogDir();
  } catch (error) {
    toast.error("The log folder could not be opened", {
      description: toAppError(error).message,
    });
  }
}

/**
 * Settings › Logs & privacy (plan 6.2, 6.3 `privacy`): what blurrd keeps
 * on this PC and sends nowhere (the statement of `docs/privacy.md`), the
 * log level and log folder, and the cache of analyses with a way to clear
 * it. blurrd has no network features, so there is nothing to switch off.
 */
export function PrivacySettings() {
  const privacy = useSettingsStore((state) => state.privacy);
  const update = useSettingsStore((state) => state.updatePrivacy);
  const queryClient = useQueryClient();
  const cache = useQuery({ queryKey: CACHE_KEY, queryFn: getCacheInfo, staleTime: 0 });
  const [confirming, setConfirming] = useState(false);
  const clear = useMutation({
    mutationFn: () => clearCache(useQueueStore.getState().items.map((item) => item.path)),
    onSuccess: (info) => {
      queryClient.setQueryData(CACHE_KEY, info);
      setConfirming(false);
      toast.success("Cached analyses cleared");
    },
    onError: (error) => {
      toast.error("The cache could not be cleared", { description: toAppError(error).message });
    },
  });
  const levelLabel = useId();
  const logLabel = useId();
  const logPath = useId();
  const cacheLabel = useId();
  const cacheText = useId();
  const info = cache.data;

  return (
    <SettingsPage title="Logs & privacy" description="What blurrd keeps on this PC.">
      <ReadOnlyNotice />
      <section
        aria-label="Privacy"
        className="flex flex-col gap-2 rounded-xl border border-card-border bg-card px-4 py-3"
      >
        <h2 className="text-ui-base font-medium">Everything stays on this PC</h2>
        <p className="text-ui-base leading-6 text-foreground-subtle">
          blurrd works offline. It makes no network connections and sends no telemetry. Your
          recordings, their analyses and the verification reports stay on this PC; reports and logs
          never contain the text OCR read.
        </p>
      </section>
      <SettingsGroup>
        <SettingsRow
          label="Log level"
          labelId={levelLabel}
          description="How much the log records. Logs hold file names, frame numbers, timings and errors, never recognised text. Applies at once."
          control={
            <Select
              value={privacy.log_level}
              onValueChange={(value) => update({ log_level: value as LogLevel })}
            >
              <SelectTrigger size="lg" aria-labelledby={levelLabel} className="w-full">
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                {LOG_LEVELS.map(({ value, label }) => (
                  <SelectItem key={value} value={value}>
                    {label}
                  </SelectItem>
                ))}
              </SelectContent>
            </Select>
          }
        />
        <SettingsRow
          label="Log folder"
          labelId={logLabel}
          description={
            <span id={logPath} title={info?.log_dir} className="block truncate font-mono">
              {info?.log_dir ?? "…"}
            </span>
          }
          control={
            <Button
              variant="outline"
              size="lg"
              aria-describedby={`${logLabel} ${logPath}`}
              onClick={() => void openLogs()}
            >
              <FolderOpen aria-hidden />
              Open folder
            </Button>
          }
        />
      </SettingsGroup>
      <SettingsGroup>
        <SettingsRow
          label="Cached analyses"
          labelId={cacheLabel}
          description={
            <span id={cacheText} className="flex flex-col gap-0.5">
              <span data-testid="cache-summary">
                {info
                  ? info.projects === 0
                    ? "No cached analyses."
                    : `${info.projects} ${info.projects === 1 ? "recording" : "recordings"}, ${formatBytes(info.project_bytes)}: boxes, edits, thumbnails and reports.`
                  : "Reading the cache…"}
              </span>
              <span title={info?.cache_dir} className="block truncate font-mono">
                {info?.cache_dir ?? ""}
              </span>
            </span>
          }
          control={
            <Button
              variant="outline"
              size="lg"
              aria-describedby={`${cacheLabel} ${cacheText}`}
              disabled={!info || info.projects === 0 || clear.isPending}
              onClick={() => setConfirming(true)}
            >
              <Trash2 aria-hidden />
              Clear…
            </Button>
          }
        />
      </SettingsGroup>
      <Dialog open={confirming} onOpenChange={setConfirming}>
        <DialogContent>
          <DialogHeader>
            <DialogTitle>Clear the cached analyses?</DialogTitle>
            <DialogDescription>
              The boxes, manual edits and verification reports of every recording that is not in the
              queue are deleted; those recordings are analysed again when you add them. Exported
              files are not touched.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <DialogClose asChild>
              <Button variant="outline" size="lg">
                Cancel
              </Button>
            </DialogClose>
            <Button
              variant="destructive"
              size="lg"
              disabled={clear.isPending}
              onClick={() => clear.mutate()}
            >
              Clear
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </SettingsPage>
  );
}
