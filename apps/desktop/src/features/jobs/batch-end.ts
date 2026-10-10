/**
 * What happens when a batch ends: one system notification sums it up if
 * the window is in the background (design 8.3), and the output folder
 * opens when the user asked for that (Settings › General).
 */
import { toast } from "sonner";

import { toAppError } from "@/ipc/errors";
import { isWindowFocused, revealFile, showNotification } from "@/ipc/system";
import { batchNotice, type Batch } from "@/lib/batch";
import { logWarning } from "@/lib/log";
import { useSettingsStore } from "@/store/settings";

/** Shows a system notification unless the window has the focus (the toast says it then). */
async function notifyInBackground(title: string, body: string): Promise<void> {
  try {
    if (await isWindowFocused()) return;
    await showNotification(title, body);
  } catch (reason) {
    logWarning("showing a notification failed", reason);
  }
}

/** Opens Explorer at an exported file. */
export async function showInFolder(path: string): Promise<void> {
  try {
    await revealFile(path);
  } catch (reason) {
    logWarning("revealing a file failed", reason);
    toast.error("The folder could not be opened", { description: toAppError(reason).message });
  }
}

/** Notifies about an ended batch and opens the folder of its last export, as the settings say. */
export function batchEnded(batch: Batch): void {
  const { general } = useSettingsStore.getState();
  const notice = batchNotice(batch);
  if (notice && general.notify) void notifyInBackground(notice.title, notice.body);
  const last = batch.ended
    .flatMap((outcome) => (outcome.output === null ? [] : [outcome.output]))
    .at(-1);
  if (general.open_folder_after_export && last !== undefined) void showInFolder(last);
}
