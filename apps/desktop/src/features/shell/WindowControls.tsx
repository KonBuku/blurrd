import { getCurrentWindow } from "@tauri-apps/api/window";
import { Copy, Minus, Square, X } from "lucide-react";
import { useEffect, useState } from "react";

import { logWarning } from "@/lib/log";

const buttonClass =
  "grid h-10 w-11 place-items-center text-neutral-700 outline-none hover:bg-neutral-200 focus-visible:ring-2 focus-visible:ring-sky-500 focus-visible:ring-inset dark:text-neutral-300 dark:hover:bg-neutral-800";

/** Click handler that runs a window call and logs its failure. */
function run(action: string, call: () => Promise<void>) {
  return () => {
    call().catch((error: unknown) => logWarning(`${action} failed`, error));
  };
}

/**
 * Minimize, maximize/restore and close buttons of the custom title bar
 * (the window has no native decorations, decision D6).
 *
 * The maximize button follows the real window state, which also changes
 * through double-clicks on the title bar, drag-to-edge snapping and
 * Win+arrow keys, so it listens to resize events instead of tracking its
 * own clicks.
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    const appWindow = getCurrentWindow();
    let disposed = false;
    let unlisten: (() => void) | undefined;

    const sync = () => {
      appWindow
        .isMaximized()
        .then((value) => {
          if (!disposed) setMaximized(value);
        })
        .catch((error: unknown) => logWarning("reading the window state failed", error));
    };

    sync();
    appWindow
      .onResized(sync)
      .then((stop) => {
        if (disposed) stop();
        else unlisten = stop;
      })
      .catch((error: unknown) => logWarning("listening to window resizes failed", error));

    return () => {
      disposed = true;
      unlisten?.();
    };
  }, []);

  const appWindow = getCurrentWindow();

  return (
    <div className="flex h-full items-stretch">
      <button
        type="button"
        aria-label="Minimize"
        title="Minimize"
        className={buttonClass}
        onClick={run("minimizing the window", () => appWindow.minimize())}
      >
        <Minus className="size-4" aria-hidden />
      </button>
      <button
        type="button"
        aria-label={maximized ? "Restore" : "Maximize"}
        title={maximized ? "Restore" : "Maximize"}
        className={buttonClass}
        onClick={run("maximizing the window", () => appWindow.toggleMaximize())}
      >
        {maximized ? (
          <Copy className="size-3.5 -scale-x-100" aria-hidden />
        ) : (
          <Square className="size-3.5" aria-hidden />
        )}
      </button>
      <button
        type="button"
        aria-label="Close"
        title="Close"
        className={`${buttonClass} hover:bg-red-600 hover:text-white dark:hover:bg-red-600 dark:hover:text-white`}
        onClick={run("closing the window", () => appWindow.close())}
      >
        <X className="size-4" aria-hidden />
      </button>
    </div>
  );
}
