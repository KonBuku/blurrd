import { beforeEach, describe, expect, it } from "vitest";

import { fakeMedia } from "@/ipc/mock/media";
import { DEFAULT_JOB_SETTINGS } from "@/lib/job-settings";
import { pathKey } from "@/lib/media";

import { settingsOf, targetsOf, useJobSettingsStore } from "./job-settings";
import { useQueueStore } from "./queue";

const queue = () => useQueueStore.getState();

function add(...paths: string[]) {
  const pending = useQueueStore.getState().addPending(paths);
  useQueueStore.getState().resolvePending(
    pending,
    pending.map((path) => ({ path, media: fakeMedia(path) })),
  );
}

describe("job settings store", () => {
  beforeEach(() => {
    useQueueStore.setState(useQueueStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
  });

  it("targets the selection, the defaults without one, or everything", () => {
    add("D:\\a.mp4", "D:\\b.mp4");
    expect(targetsOf(queue(), "selected")).toEqual({ ids: [], defaults: true });
    queue().select(pathKey("D:\\b.mp4"), "replace");
    expect(targetsOf(queue(), "selected")).toEqual({
      ids: [pathKey("D:\\b.mp4")],
      defaults: false,
    });
    expect(targetsOf(queue(), "all")).toEqual({
      ids: [pathKey("D:\\a.mp4"), pathKey("D:\\b.mp4")],
      defaults: true,
    });
  });

  it("changes one section field by field and forgets removed files", () => {
    add("D:\\a.mp4");
    const id = pathKey("D:\\a.mp4");
    const store = useJobSettingsStore.getState();
    store.updateSection({ ids: [id], defaults: false }, "blur", (blur) => ({ ...blur, radius: 2 }));
    const state = useJobSettingsStore.getState();
    expect(settingsOf(state, id)).toEqual({
      ...DEFAULT_JOB_SETTINGS,
      blur: { ...DEFAULT_JOB_SETTINGS.blur, radius: 2 },
    });
    expect(settingsOf(state, null)).toBe(DEFAULT_JOB_SETTINGS);
    useQueueStore.getState().removeAll();
    expect(useJobSettingsStore.getState().files).toEqual({});
  });
});
