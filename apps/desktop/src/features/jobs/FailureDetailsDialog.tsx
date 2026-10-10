import { Check, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { create } from "zustand";

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
import { logWarning } from "@/lib/log";

/** A failure as the details dialog shows it. */
export interface FailureDetails {
  /** What failed, for example "match_bind_02.mp4 could not be exported". */
  readonly title: string;
  /** The one-line message. */
  readonly message: string;
  /** Technical detail for bug reports, if the backend gave any. */
  readonly detail: string | null;
}

/** The failure whose details are shown, if any. */
export const useFailureDetailsStore = create<{ readonly shown: FailureDetails | null }>()(() => ({
  shown: null,
}));

/** Opens the details dialog for `failure`. */
export function showFailureDetails(failure: FailureDetails): void {
  useFailureDetailsStore.setState({ shown: failure });
}

/** The text "Copy details" puts on the clipboard. */
export function failureReport({ title, message, detail }: FailureDetails): string {
  return [title, message, detail].filter((part): part is string => part !== null).join("\r\n\r\n");
}

/** How long the copy button shows its confirmation, in milliseconds. */
const COPIED_MS = 1500;

/**
 * The failure details dialog (design 8.3): the message, the technical
 * detail in a read-only, scrollable monospace field (focusable, so it
 * scrolls and selects from the keyboard) and a button that copies both
 * for a bug report. Opened from the failure toast and the queue menu.
 */
export function FailureDetailsDialog() {
  const shown = useFailureDetailsStore((state) => state.shown);
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return undefined;
    const timer = setTimeout(() => setCopied(false), COPIED_MS);
    return () => clearTimeout(timer);
  }, [copied]);

  const copy = async () => {
    if (!shown) return;
    try {
      await navigator.clipboard.writeText(failureReport(shown));
      setCopied(true);
    } catch (reason) {
      logWarning("copying the failure details failed", reason);
    }
  };

  return (
    <Dialog
      open={shown !== null}
      onOpenChange={(open) => {
        if (!open) {
          useFailureDetailsStore.setState({ shown: null });
          setCopied(false);
        }
      }}
    >
      <DialogContent className="max-w-lg" showClose={false}>
        <DialogHeader>
          <DialogTitle>{shown?.title}</DialogTitle>
          <DialogDescription>{shown?.message}</DialogDescription>
        </DialogHeader>
        {shown?.detail ? (
          <textarea
            readOnly
            aria-label="Technical details"
            rows={8}
            value={shown.detail}
            className="max-h-60 w-full resize-none rounded-lg border border-input-border bg-input p-3 font-mono text-ui-sm break-all text-foreground focus-visible:border-input-border-focused focus-visible:bg-input-focused"
          />
        ) : (
          <p className="text-ui-sm text-foreground-subtle">No technical details were reported.</p>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => void copy()}>
            {copied ? <Check aria-hidden /> : <Copy aria-hidden />}
            {copied ? "Copied" : "Copy details"}
          </Button>
          <DialogClose asChild>
            <Button>Close</Button>
          </DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
