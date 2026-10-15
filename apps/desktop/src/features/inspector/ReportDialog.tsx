import { useQueryClient } from "@tanstack/react-query";
import { Eye, EyeOff } from "lucide-react";
import { useEffect, useState } from "react";
import { toast } from "sonner";

import type { ProjectSummary } from "@/bindings/ProjectSummary";
import type { Region } from "@/bindings/Region";
import type { RegionVerify } from "@/bindings/RegionVerify";
import type { VerifyReport } from "@/bindings/VerifyReport";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { RegionChip } from "@/components/ui/region-chip";
import { acceptReport } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { leakCropUrl } from "@/ipc/media";
import { formatTimecode } from "@/lib/frames";
import { logWarning } from "@/lib/log";
import { baseName, formatCount } from "@/lib/media";
import { REGIONS } from "@/lib/regions";
import {
  STATUS_LABELS,
  ZONE_LABELS,
  countedLeaks,
  outOfScope,
  type IndexedLeak,
} from "@/lib/verify-report";
import { useQueueStore, type QueueItem } from "@/store/queue";

import { ResultBadge } from "./ResultBadge";

export interface ReportDialogProps {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly item: QueueItem;
  readonly project: ProjectSummary;
  readonly report: VerifyReport;
}

/**
 * The verification report (design 8.4, plan 6.6): stat tiles, the
 * positive control per region, the leaks with their crops, the items OCR
 * could not read and the names outside every region. Crops are shown as
 * the backend blurs them; "Reveal" re-decodes one frame locally on
 * request and the clear image is dropped when the dialog closes. For an
 * output named `_UNVERIFIED`, "Accept and rename" accepts the open leaks
 * and renames the file (plan 7.2).
 */
export function ReportDialog({ open, onOpenChange, item, project, report }: ReportDialogProps) {
  const [num, den] = project.media.fps;
  const time = (frame: number) => formatTimecode((frame * den) / num);
  const { summary } = report;
  const leaks = countedLeaks(report.leaks);
  const outside = outOfScope(report.leaks);
  const regions = Object.entries(report.per_region) as [Region, RegionVerify][];
  const unverifiable = regions.filter(([, counts]) => counts.unverifiable_items > 0);
  const title = item.output ? baseName(item.output) : item.name;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[calc(100%-2rem)] grid-rows-[auto_minmax(0,1fr)_auto] sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex flex-wrap items-center gap-2 pr-8">
            <span className="min-w-0 break-all">Verification report · {title}</span>
            <ResultBadge summary={summary} />
          </DialogTitle>
          <DialogDescription>
            Revision {report.plan_revision} ·{" "}
            {report.coverage === "EveryFrame" ? "every frame" : "frames with HUD activity"}. No
            recognised text is stored; crops stay blurred until you reveal one.
          </DialogDescription>
        </DialogHeader>

        {/* Focusable, so that the report scrolls by keyboard. */}
        <div
          // oxlint-disable-next-line jsx-a11y/no-noninteractive-tabindex
          tabIndex={0}
          className="flex min-h-0 flex-col gap-5 overflow-y-auto pr-1"
        >
          <dl className="grid grid-cols-3 gap-px overflow-hidden rounded-xl border border-card-border bg-border sm:grid-cols-6">
            <Stat label="Frames checked" value={report.frames_checked} />
            <Stat label="Open" value={summary.open} />
            <Stat label="Fixed" value={summary.fixed} />
            <Stat label="Accepted" value={summary.accepted} />
            <Stat label="Outside regions" value={summary.out_of_scope} />
            <Stat label="Unverifiable" value={summary.unverifiable_items} />
          </dl>

          {regions.length > 0 && (
            <section aria-labelledby="report-regions" className="flex flex-col gap-2">
              <h3 id="report-regions" className="text-ui-base font-medium">
                Positive control
              </h3>
              <table className="w-full text-left text-ui-sm">
                <thead className="text-foreground-subtle">
                  <tr>
                    <th scope="col" className="py-1 font-medium">
                      Region
                    </th>
                    <th scope="col" className="py-1 text-right font-medium">
                      Names
                    </th>
                    <th scope="col" className="py-1 text-right font-medium">
                      Read on the source
                    </th>
                    <th scope="col" className="py-1 text-right font-medium">
                      Unverifiable
                    </th>
                  </tr>
                </thead>
                <tbody className="tabular-nums">
                  {regions.map(([region, counts]) => (
                    <tr key={region} className="border-t border-border">
                      <td className="py-1.5">
                        <RegionChip region={region} />
                      </td>
                      <td className="py-1.5 text-right">{formatCount(counts.items)}</td>
                      <td className="py-1.5 text-right">
                        {Math.round(counts.positive_control * 100)} %
                      </td>
                      <td className="py-1.5 text-right">
                        {formatCount(counts.unverifiable_items)}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}

          <section aria-labelledby="report-leaks" className="flex flex-col gap-2">
            <h3 id="report-leaks" className="text-ui-base font-medium">
              Leaks
            </h3>
            {leaks.length === 0 ? (
              <p className="text-ui-sm text-foreground-subtle">
                OCR read no names in the blurred regions.
              </p>
            ) : (
              <table className="w-full text-left text-ui-sm">
                <thead className="text-foreground-subtle">
                  <tr>
                    <th scope="col" className="py-1 font-medium">
                      Crop
                    </th>
                    <th scope="col" className="py-1 font-medium">
                      Frames
                    </th>
                    <th scope="col" className="py-1 font-medium">
                      Region
                    </th>
                    <th scope="col" className="py-1 text-right font-medium">
                      Confidence
                    </th>
                    <th scope="col" className="py-1 text-right font-medium">
                      Status
                    </th>
                  </tr>
                </thead>
                <tbody>
                  {leaks.map((entry) => (
                    <LeakRow key={entry.index} entry={entry} projectId={project.id} time={time} />
                  ))}
                </tbody>
              </table>
            )}
          </section>

          {unverifiable.length > 0 && (
            <section aria-labelledby="report-unverifiable" className="flex flex-col gap-2">
              <h3 id="report-unverifiable" className="text-ui-base font-medium">
                Not verifiable
              </h3>
              <p className="text-ui-sm text-foreground-subtle">
                OCR could not read these names on the source either; they are covered by their boxes
                only.
              </p>
              <ul className="flex flex-col gap-1 text-ui-sm">
                {unverifiable.map(([region, counts]) => (
                  <li key={region} className="flex items-center gap-2">
                    <RegionChip region={region} />
                    {formatCount(counts.unverifiable_items)}{" "}
                    {counts.unverifiable_items === 1 ? "name" : "names"}
                  </li>
                ))}
              </ul>
            </section>
          )}

          {outside.length > 0 && (
            <section aria-labelledby="report-outside" className="flex flex-col gap-2">
              <h3 id="report-outside" className="text-ui-base font-medium">
                Outside the regions (information)
              </h3>
              <p className="text-ui-sm text-foreground-subtle">
                Names found outside every region, for example tags above players&apos; heads. They
                are out of scope, never blurred and do not change the result.
              </p>
              <ul className="flex flex-col gap-1 font-mono text-ui-sm tabular-nums">
                {outside.map(({ index, leak }) => (
                  <li key={index}>
                    {time(leak.frame_range[0])}–{time(leak.frame_range[1])}
                  </li>
                ))}
              </ul>
            </section>
          )}
        </div>

        <DialogFooter>
          {item.status.kind === "unverified" && item.output && project.verify.report && (
            <AcceptButton
              item={item}
              projectId={project.id}
              reportId={project.verify.report}
              open={summary.open}
              onDone={() => onOpenChange(false)}
            />
          )}
          <Button variant="outline" onClick={() => onOpenChange(false)}>
            Close
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** One stat tile. */
function Stat({ label, value }: { readonly label: string; readonly value: number }) {
  return (
    <div className="flex flex-col gap-0.5 bg-card px-3 py-2">
      <dt className="text-ui-xs text-foreground-subtle">{label}</dt>
      <dd className="text-ui-lg font-medium tabular-nums">{formatCount(value)}</dd>
    </div>
  );
}

interface LeakRowProps {
  readonly entry: IndexedLeak;
  readonly projectId: string;
  readonly time: (frame: number) => string;
}

/** A leak: its crop (blurred until revealed), frames, region, confidence and status. */
function LeakRow({ entry: { index, leak }, projectId, time }: LeakRowProps) {
  const [revealed, setRevealed] = useState(false);
  const image = useCropImage(leakCropUrl(projectId, index, revealed ? "clear" : "blurred"));
  const where = leak.region ? REGIONS[leak.region].label : ZONE_LABELS[leak.zone];
  return (
    <tr className="border-t border-border align-middle">
      <td className="py-1.5">
        <div className="flex items-center gap-1.5">
          <div className="grid h-8 w-24 place-items-center overflow-hidden rounded-sm bg-video-stage">
            {image && (
              <img
                src={image}
                alt={
                  revealed
                    ? `Revealed crop of the leak at ${time(leak.frame_range[0])}`
                    : `Blurred crop of the leak at ${time(leak.frame_range[0])}`
                }
                data-view={revealed ? "clear" : "blurred"}
                className="max-h-full max-w-full object-contain [image-rendering:pixelated]"
              />
            )}
          </div>
          <Button
            variant="ghost"
            size="icon-sm"
            aria-pressed={revealed}
            aria-label={revealed ? "Hide the crop" : "Reveal the crop"}
            onClick={() => setRevealed(!revealed)}
          >
            {revealed ? <EyeOff strokeWidth={1.5} /> : <Eye strokeWidth={1.5} />}
          </Button>
        </div>
      </td>
      <td className="py-1.5 font-mono tabular-nums">
        {time(leak.frame_range[0])}–{time(leak.frame_range[1])}
      </td>
      <td className="py-1.5">{leak.region ? <RegionChip region={leak.region} /> : where}</td>
      <td className="py-1.5 text-right tabular-nums">{Math.round(leak.conf * 100)} %</td>
      <td className="py-1.5 text-right">{STATUS_LABELS[leak.status]}</td>
    </tr>
  );
}

/**
 * Loads a crop into an object URL that lives only while the row shows it
 * (a revealed crop is not cached).
 */
function useCropImage(url: string): string | null {
  // The loaded image and the URL it belongs to; a new URL shows nothing until it loads.
  const [loaded, setLoaded] = useState<{ readonly url: string; readonly image: string } | null>(
    null,
  );
  useEffect(() => {
    const controller = new AbortController();
    let objectUrl: string | null = null;
    void (async () => {
      try {
        const response = await fetch(url, { signal: controller.signal });
        if (!response.ok) throw new Error(`${url}: HTTP ${response.status}`);
        objectUrl = URL.createObjectURL(await response.blob());
        setLoaded({ url, image: objectUrl });
      } catch (reason) {
        if (!controller.signal.aborted) logWarning("loading a leak crop failed", reason);
      }
    })();
    return () => {
      controller.abort();
      if (objectUrl) URL.revokeObjectURL(objectUrl);
    };
  }, [url]);
  return loaded?.url === url ? loaded.image : null;
}

interface AcceptButtonProps {
  readonly item: QueueItem;
  readonly projectId: string;
  readonly reportId: string;
  /** Leaks that are still open (they become accepted). */
  readonly open: number;
  readonly onDone: () => void;
}

/** Accepts the report and renames the `_UNVERIFIED` output. */
function AcceptButton({ item, projectId, reportId, open, onDone }: AcceptButtonProps) {
  const queryClient = useQueryClient();
  const [busy, setBusy] = useState(false);
  const accept = async () => {
    setBusy(true);
    try {
      const output = await acceptReport(projectId, reportId);
      useQueueStore
        .getState()
        .setStatus(
          item.id,
          open > 0 ? { kind: "warnings", leaks: open } : { kind: "done", verified: false },
          output,
        );
      await Promise.all([
        queryClient.invalidateQueries({ queryKey: ["verify-report", projectId] }),
        queryClient.invalidateQueries({ queryKey: ["project", item.path] }),
        queryClient.invalidateQueries({ queryKey: ["output-preview", projectId] }),
      ]);
      toast.success(`Report accepted · renamed to ${baseName(output)}`);
      onDone();
    } catch (reason) {
      toast.error("The report could not be accepted", { description: toAppError(reason).message });
    } finally {
      setBusy(false);
    }
  };
  return (
    <Button disabled={busy} aria-busy={busy} onClick={() => void accept()}>
      Accept and rename
    </Button>
  );
}
