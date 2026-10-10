import { openUrl } from "@tauri-apps/plugin-opener";
import { toast } from "sonner";

import { toAppError } from "@/ipc/errors";
import { logWarning } from "@/lib/log";

/** The project's home. */
export const REPOSITORY_URL = "https://github.com/KonBuku/blurrd";

/** Where users report problems (the issue template chooser). */
export const ISSUES_URL = `${REPOSITORY_URL}/issues/new/choose`;

/**
 * Opens a page of the project in the default browser. The app may open
 * only URLs below {@link REPOSITORY_URL} (capability `opener:allow-open-url`).
 */
export async function openProjectPage(url: string): Promise<void> {
  try {
    await openUrl(url);
  } catch (reason) {
    logWarning("opening a project page failed", reason);
    toast.error("The page could not be opened", { description: toAppError(reason).message });
  }
}
