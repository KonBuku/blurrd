import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

import type { ProbeOutcome } from "@/bindings/ProbeOutcome";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";

import { closeRemovedProjects } from "./project-lifetime";
import { useQueueStore } from "./queue";

function ok(path: string): ProbeOutcome {
  return { path, media: fakeMedia(path) };
}

function add(...paths: string[]) {
  const pending = useQueueStore.getState().addPending(paths);
  useQueueStore.getState().resolvePending(pending, pending.map(ok));
}

function idOf(path: string): string {
  const item = useQueueStore.getState().items.find((entry) => entry.path === path);
  if (!item) throw new Error(`${path} is not queued`);
  return item.id;
}

describe("project lifetime", () => {
  let backend: MockBackend;
  let queryClient: QueryClient;
  let stop: () => void;

  beforeEach(() => {
    backend = installMockBackend();
    useQueueStore.setState(useQueueStore.getInitialState());
    queryClient = new QueryClient();
    stop = closeRemovedProjects(queryClient);
  });

  afterEach(() => stop());

  const closed = () =>
    backend.calls
      .filter((call) => call.cmd === "close_project")
      .map((call) => (call.args as { projectId: string }).projectId);

  it("closes the project of a removed file and drops its cached reads", () => {
    add("D:\\a.mp4", "D:\\b.mp4", "D:\\c.mp4");
    queryClient.setQueryData(["project", "D:\\a.mp4"], { id: "pa" });
    queryClient.setQueryData(["tracks", "pa"], { revision: 1 });
    queryClient.setQueryData(["project", "D:\\b.mp4"], { id: "pb" });
    // c.mp4 was never opened: there is nothing to close.

    useQueueStore.getState().remove([idOf("D:\\a.mp4"), idOf("D:\\c.mp4")]);

    expect(closed()).toEqual(["pa"]);
    expect(queryClient.getQueryData(["project", "D:\\a.mp4"])).toBeUndefined();
    expect(queryClient.getQueryData(["tracks", "pa"])).toBeUndefined();
    expect(queryClient.getQueryData(["project", "D:\\b.mp4"])).toEqual({ id: "pb" });
  });

  it("keeps a project another queued path still uses", () => {
    add("D:\\a.mp4", "E:\\copy of a.mp4");
    queryClient.setQueryData(["project", "D:\\a.mp4"], { id: "same" });
    queryClient.setQueryData(["project", "E:\\copy of a.mp4"], { id: "same" });

    useQueueStore.getState().remove([idOf("D:\\a.mp4")]);
    expect(closed()).toEqual([]);

    useQueueStore.getState().removeAll();
    expect(closed()).toEqual(["same"]);
  });
});
