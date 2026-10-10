/**
 * The operating system around the window: the taskbar button's progress,
 * system notifications and Explorer (plan 9.1 plugins: window API,
 * notification, opener).
 */
import { getCurrentWindow, ProgressBarStatus } from "@tauri-apps/api/window";
import {
  isPermissionGranted,
  requestPermission,
  sendNotification,
} from "@tauri-apps/plugin-notification";
import { revealItemInDir } from "@tauri-apps/plugin-opener";

import type { TaskbarState, TaskbarStatus } from "@/lib/batch";

const STATUS: Readonly<Record<TaskbarStatus, ProgressBarStatus>> = {
  none: ProgressBarStatus.None,
  normal: ProgressBarStatus.Normal,
  paused: ProgressBarStatus.Paused,
  error: ProgressBarStatus.Error,
};

/** Shows `state` on the window's taskbar button. */
export function setTaskbarProgress(state: TaskbarState): Promise<void> {
  return getCurrentWindow().setProgressBar({
    status: STATUS[state.status],
    progress: state.progress,
  });
}

/** Whether the main window has the keyboard focus. */
export function isWindowFocused(): Promise<boolean> {
  return getCurrentWindow().isFocused();
}

/**
 * Shows a system notification, asking for the permission first when the
 * system wants that (Windows grants it to installed apps). Resolves to
 * whether it was shown.
 */
export async function showNotification(title: string, body: string): Promise<boolean> {
  const granted = (await isPermissionGranted()) || (await requestPermission()) === "granted";
  if (granted) sendNotification({ title, body });
  return granted;
}

/** Opens the folder of `path` in Explorer with the file selected. */
export function revealFile(path: string): Promise<void> {
  return revealItemInDir(path);
}
