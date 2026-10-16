/**
 * Closes the backend projects of files removed from the queue
 * (`close_project`): the backend stops serving their videos and closes
 * their Exact decoder, and the cached reads of the project are dropped,
 * so a file added again is opened afresh.
 */
import type { QueryClient } from "@tanstack/react-query";

import type { ProjectSummary } from "@/bindings/ProjectSummary";
import { closeProject, openingProject } from "@/ipc/commands";
import { toAppError } from "@/ipc/errors";
import { logWarning } from "@/lib/log";

import { useQueueStore, type QueueItem } from "./queue";

function close(id: string) {
  closeProject(id).catch((reason: unknown) => {
    logWarning("closing a project failed", toAppError(reason).message);
  });
}

/**
 * Closes the project a removed file's `open_project` call answers once
 * it settles, unless the file was queued again or another queued path
 * uses the same project by then.
 */
function closeWhenOpened(path: string, opened: Promise<ProjectSummary>, queryClient: QueryClient) {
  opened.then(
    ({ id }) => {
      const items = useQueueStore.getState().items;
      if (items.some((item) => item.path === path)) return;
      const inUse = items.some(
        (item) => queryClient.getQueryData<ProjectSummary>(["project", item.path])?.id === id,
      );
      if (!inUse) close(id);
    },
    () => undefined,
  );
}

/**
 * Closes the projects of the files in `before` that are not in `after`.
 * A project still opened through another queued path (the same video
 * twice) stays open; a file whose project is still being opened has it
 * closed once the backend answers. Answers the closed project ids.
 */
export function closeRemoved(
  before: readonly QueueItem[],
  after: readonly QueueItem[],
  queryClient: QueryClient,
): string[] {
  const idOf = (path: string) =>
    queryClient.getQueryData<ProjectSummary>(["project", path])?.id ?? null;
  const keptPaths = new Set(after.map((item) => item.path));
  const keptIds = new Set(after.map((item) => idOf(item.path)));
  const closed: string[] = [];
  for (const { path } of before) {
    if (keptPaths.has(path)) continue;
    const id = idOf(path);
    queryClient.removeQueries({ queryKey: ["project", path], exact: true });
    if (id === null) {
      // The backend registers the project when the call returns.
      const opened = openingProject(path);
      if (opened) closeWhenOpened(path, opened, queryClient);
      continue;
    }
    if (keptIds.has(id) || closed.includes(id)) continue;
    closed.push(id);
    queryClient.removeQueries({ predicate: (query) => query.queryKey[1] === id });
    close(id);
  }
  return closed;
}

/**
 * Closes the project of every file the queue loses from now on. Returns
 * a function that stops watching.
 */
export function closeRemovedProjects(queryClient: QueryClient): () => void {
  return useQueueStore.subscribe((state, previous) => {
    if (state.items !== previous.items) closeRemoved(previous.items, state.items, queryClient);
  });
}
