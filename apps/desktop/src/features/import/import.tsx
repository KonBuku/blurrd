/**
 * Adding recordings to the queue: from the file and folder dialogs, from
 * drops on the window and from files passed to a second app start. The
 * paths are read by the backend (`probe_media`); files it cannot use are
 * listed in one toast.
 */
import { open } from "@tauri-apps/plugin-dialog";
import { toast } from "sonner";

import type { AppError } from "@/bindings/AppError";
import { probeMedia } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { logWarning } from "@/lib/log";
import { baseName, formatCount } from "@/lib/media";
import { useQueueStore } from "@/store/queue";

/** Containers offered by the file dialog (design 8.2: MP4 · MOV · MKV). */
export const VIDEO_EXTENSIONS: readonly string[] = ["mp4", "mov", "mkv"];

/** Rejected files listed by name in the toast; the rest are counted. */
const LISTED_REJECTIONS = 4;

/** Short reason for a file the backend rejected. */
function rejectionReason(error: AppError): string {
  switch (error.kind) {
    case "UnsupportedMedia":
      return "not a supported video";
    case "NotFound":
      return "file not found";
    default:
      return error.message;
  }
}

/** Lists rejected files in one warning toast. */
function reportRejected(rejected: readonly { path: string; error: AppError }[]): void {
  if (rejected.length === 0) return;
  const title =
    rejected.length === 1
      ? "1 file can't be added"
      : `${formatCount(rejected.length)} files can't be added`;
  const rest = rejected.length - LISTED_REJECTIONS;
  toast.warning(title, {
    duration: 8000,
    description: (
      <ul className="flex flex-col">
        {rejected.slice(0, LISTED_REJECTIONS).map(({ path, error }) => (
          <li key={path} className="truncate">
            {baseName(path)} — {rejectionReason(error)}
          </li>
        ))}
        {rest > 0 && <li>and {formatCount(rest)} more</li>}
      </ul>
    ),
  });
}

/**
 * Adds paths (files or folders) to the queue. Each new path shows as
 * "Reading…" at once; the probe then replaces it with the videos found.
 * Paths already in the queue are skipped.
 */
export async function importPaths(paths: readonly string[]): Promise<void> {
  if (paths.length === 0) return;
  const queue = useQueueStore.getState();
  const pending = queue.addPending(paths);
  if (pending.length === 0) {
    toast.info(
      paths.length === 1 ? "Already in the queue" : "All of these are already in the queue",
    );
    return;
  }

  let outcomes;
  try {
    outcomes = await probeMedia(pending);
  } catch (reason) {
    useQueueStore.getState().dropPending(pending);
    const error = toAppError(reason);
    logWarning("reading the added files failed", error.detail ?? error.message);
    toast.error("The files could not be read", { description: error.message });
    return;
  }

  const { rejected, duplicates } = useQueueStore.getState().resolvePending(pending, outcomes);
  reportRejected(rejected);
  if (duplicates.length > 0) {
    toast.info(
      duplicates.length === 1
        ? "1 video was already in the queue"
        : `${formatCount(duplicates.length)} videos were already in the queue`,
    );
  }
}

/** Opens the file dialog and adds the chosen videos. */
export async function browseFiles(): Promise<void> {
  const picked = await pick(() =>
    open({
      title: "Add recordings",
      multiple: true,
      filters: [
        { name: "Videos", extensions: [...VIDEO_EXTENSIONS] },
        { name: "All files", extensions: ["*"] },
      ],
    }),
  );
  await importPaths(picked);
}

/** Opens the folder dialog and adds the videos inside the chosen folders. */
export async function browseFolder(): Promise<void> {
  const picked = await pick(() =>
    open({ title: "Add folders with recordings", directory: true, multiple: true }),
  );
  await importPaths(picked);
}

/** Runs a dialog; a canceled or failed dialog yields no paths. */
async function pick(dialog: () => Promise<string[] | null>): Promise<string[]> {
  try {
    return (await dialog()) ?? [];
  } catch (reason) {
    logWarning("the file dialog failed", reason);
    toast.error("The file dialog could not be opened");
    return [];
  }
}
