import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ArrowDownToLine } from "lucide-react";
import { useEffect, useState } from "react";

import { logWarning } from "@/lib/log";
import { formatCount } from "@/lib/media";

import { importPaths } from "./import";

/** Payload of the `second-instance` event sent by the Rust side. */
interface SecondInstancePayload {
  /** Absolute paths passed on the command line of the second start. */
  readonly paths: readonly string[];
}

/** Text of the drop pill for `count` dragged items. */
export function dropHint(count: number): string {
  if (count === 0) return "Drop to add recordings";
  return count === 1 ? "Drop to add 1 item" : `Drop to add ${formatCount(count)} items`;
}

/**
 * The window-wide drop target (design 8.1): while files are dragged over
 * the window, the window dims behind a frosted veil of the shell colour,
 * a dashed frame in the accent marks the whole window as the target and a
 * card in the popover style says how many items a drop adds; dropping
 * adds them to the queue. Files passed to a second start of the app are
 * added the same way.
 */
export function DropOverlay() {
  const [dragged, setDragged] = useState<number | null>(null);

  useEffect(() => {
    let mounted = true;
    const stops: (() => void)[] = [];
    // Subscriptions resolve asynchronously; one that resolves after the
    // unmount is stopped at once.
    const keep = (stop: () => void) => {
      if (mounted) stops.push(stop);
      else stop();
    };
    const fail = (what: string) => (reason: unknown) =>
      logWarning(`could not listen for ${what}`, reason);

    getCurrentWebview()
      .onDragDropEvent(({ payload }) => {
        switch (payload.type) {
          case "enter":
            setDragged(payload.paths.length);
            break;
          case "over":
            break;
          case "drop":
            setDragged(null);
            void importPaths(payload.paths);
            break;
          case "leave":
            setDragged(null);
            break;
        }
      })
      .then(keep, fail("dropped files"));
    listen<SecondInstancePayload>("second-instance", ({ payload }) => {
      void importPaths(payload.paths);
    }).then(keep, fail("files from a second start"));

    return () => {
      mounted = false;
      for (const stop of stops) stop();
    };
  }, []);

  if (dragged === null) return null;
  return (
    <div
      data-testid="drop-overlay"
      className="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-background-win-alt/75 backdrop-blur-sm duration-100 animate-in fade-in-0"
    >
      <div
        aria-hidden
        className="absolute inset-2 rounded-2xl border-2 border-dashed border-accent-strong/60"
      />
      <output className="flex flex-col items-center gap-3 rounded-2xl border border-popover-border bg-popover px-8 py-6 text-center text-popover-foreground shadow-md duration-150 animate-in zoom-in-95">
        <span className="grid size-10 place-items-center rounded-xl bg-surface text-foreground">
          <ArrowDownToLine aria-hidden className="size-5" />
        </span>
        <span className="flex flex-col gap-0.5">
          <span className="text-ui-base font-medium">{dropHint(dragged)}</span>
          <span className="text-ui-sm text-foreground-subtle">
            MP4 · MOV · MKV — folders add the videos inside
          </span>
        </span>
      </output>
    </div>
  );
}
