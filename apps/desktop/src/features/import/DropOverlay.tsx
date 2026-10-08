import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { Hand } from "lucide-react";
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
 * the window, an accent veil with a pill says how many items a drop adds;
 * dropping adds them to the queue. Files passed to a second start of the
 * app are added the same way.
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
      className="pointer-events-none fixed inset-0 z-50 flex items-center justify-center bg-accent/55 backdrop-blur-sm"
    >
      <output className="flex items-center gap-2 rounded-full border border-border bg-accent px-4 py-2 text-ui-base text-foreground shadow-sm">
        <Hand aria-hidden className="size-4" />
        <span>{dropHint(dragged)}</span>
      </output>
    </div>
  );
}
