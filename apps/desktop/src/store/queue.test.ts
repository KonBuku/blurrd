import { beforeEach, describe, expect, it } from "vitest";

import type { ProbeOutcome } from "@/bindings/ProbeOutcome";
import { fakeMedia } from "@/ipc/mock/media";

import { useQueueStore, visibleItems } from "./queue";

/** A successful probe outcome. */
function ok(path: string): ProbeOutcome {
  return { path, media: fakeMedia(path) };
}

/** Adds files as if the probe accepted all of them. */
function add(...paths: string[]) {
  const queue = useQueueStore.getState();
  const pending = queue.addPending(paths);
  return useQueueStore.getState().resolvePending(pending, pending.map(ok));
}

function selected() {
  return [...useQueueStore.getState().selected].toSorted();
}

function names() {
  return useQueueStore.getState().items.map((item) => item.name);
}

describe("queue store", () => {
  beforeEach(() => {
    useQueueStore.setState(useQueueStore.getInitialState());
  });

  it("shows new paths as being read, then replaces them with the probed videos", () => {
    const pending = useQueueStore.getState().addPending(["D:\\a.mp4", "D:\\Folder"]);
    expect(useQueueStore.getState().items.map((item) => item.status)).toEqual([
      { kind: "probing", folder: false },
      { kind: "probing", folder: true },
    ]);

    const result = useQueueStore.getState().resolvePending(pending, [
      ok("D:\\a.mp4"),
      ok("D:\\Folder\\b.mp4"),
      ok("D:\\Folder\\c.mkv"),
      {
        path: "D:\\Folder\\x.txt",
        error: { kind: "UnsupportedMedia", message: "", detail: null },
      },
    ]);

    expect(names()).toEqual(["a.mp4", "b.mp4", "c.mkv"]);
    expect(useQueueStore.getState().items.every((item) => item.status.kind === "queued")).toBe(
      true,
    );
    expect(useQueueStore.getState().items[0]?.media?.width).toBeGreaterThan(0);
    expect(result.added).toHaveLength(3);
    expect(result.rejected.map((outcome) => outcome.path)).toEqual(["D:\\Folder\\x.txt"]);
  });

  it("inserts the probed videos where their placeholders were", () => {
    add("D:\\first.mp4");
    const pending = useQueueStore.getState().addPending(["D:\\Folder"]);
    add("D:\\last.mp4");
    useQueueStore.getState().resolvePending(pending, [ok("D:\\Folder\\middle.mp4")]);
    expect(names()).toEqual(["first.mp4", "middle.mp4", "last.mp4"]);
  });

  it("skips paths that are already queued, in any spelling", () => {
    add("D:\\Clips\\a.mp4");
    expect(useQueueStore.getState().addPending(["d:/clips/A.MP4"])).toEqual([]);

    const pending = useQueueStore.getState().addPending(["D:\\Clips"]);
    const result = useQueueStore
      .getState()
      .resolvePending(pending, [ok("D:\\Clips\\a.mp4"), ok("D:\\Clips\\b.mp4")]);
    expect(result.duplicates).toEqual(["D:\\Clips\\a.mp4"]);
    expect(names()).toEqual(["a.mp4", "b.mp4"]);
  });

  it("drops placeholders when the probe fails", () => {
    add("D:\\a.mp4");
    const pending = useQueueStore.getState().addPending(["D:\\b.mp4"]);
    useQueueStore.getState().dropPending(pending);
    expect(names()).toEqual(["a.mp4"]);
  });

  it("selects with click, Ctrl and Shift semantics", () => {
    add("D:\\1.mp4", "D:\\2.mp4", "D:\\3.mp4", "D:\\4.mp4");
    const [one, two, , four] = useQueueStore.getState().items.map((item) => item.id);
    const { select } = useQueueStore.getState();

    select(two ?? "", "replace");
    select(four ?? "", "range");
    expect(selected()).toHaveLength(3);
    expect(useQueueStore.getState().anchor).toBe(two);
    expect(useQueueStore.getState().focused).toBe(four);

    select(one ?? "", "toggle");
    expect(selected()).toHaveLength(4);
    select(one ?? "", "toggle");
    expect(selected()).toHaveLength(3);

    select(one ?? "", "replace");
    expect(selected()).toEqual([one]);
  });

  it("removes the selection and keeps selection state consistent", () => {
    add("D:\\1.mp4", "D:\\2.mp4", "D:\\3.mp4");
    const [one, two] = useQueueStore.getState().items.map((item) => item.id);
    useQueueStore.getState().select(one ?? "", "replace");
    useQueueStore.getState().select(two ?? "", "toggle");
    useQueueStore.getState().removeSelected();
    expect(names()).toEqual(["3.mp4"]);
    expect(useQueueStore.getState().selected.size).toBe(0);
    expect(useQueueStore.getState().focused).toBeNull();
  });

  it("clears finished files and removes everything", () => {
    add("D:\\a.mp4", "D:\\b.mp4");
    const items = useQueueStore.getState().items;
    useQueueStore.setState({
      items: items.map((item, index) =>
        index === 0 ? { ...item, status: { kind: "done", verified: true } } : item,
      ),
    });
    useQueueStore.getState().removeFinished();
    expect(names()).toEqual(["b.mp4"]);
    useQueueStore.getState().removeAll();
    expect(names()).toEqual([]);
  });

  it("sorts by name (natural order), date added and status", () => {
    add("D:\\clip10.mp4", "D:\\Clip2.mp4", "D:\\clip1.mp4");
    useQueueStore.getState().sortBy("name");
    expect(names()).toEqual(["clip1.mp4", "Clip2.mp4", "clip10.mp4"]);
    useQueueStore.getState().sortBy("added");
    expect(names()).toEqual(["clip10.mp4", "Clip2.mp4", "clip1.mp4"]);

    const items = useQueueStore.getState().items;
    useQueueStore.setState({
      items: items.map((item) =>
        item.name === "clip1.mp4"
          ? { ...item, status: { kind: "failed", reason: "x", detail: null } }
          : item,
      ),
    });
    useQueueStore.getState().sortBy("status");
    expect(names()[0]).toBe("clip1.mp4");
  });

  it("filters by file name and selects only what the filter shows", () => {
    add("D:\\ascent_01.mp4", "D:\\bind_02.mp4", "D:\\ascent_03.mp4");
    useQueueStore.getState().setFilter("  ASCENT ");
    const { items, filter } = useQueueStore.getState();
    expect(visibleItems(items, filter).map((item) => item.name)).toEqual([
      "ascent_01.mp4",
      "ascent_03.mp4",
    ]);
    useQueueStore.getState().selectAll();
    expect(useQueueStore.getState().selected.size).toBe(2);

    // Narrowing the filter drops hidden items from the selection.
    useQueueStore.getState().setFilter("ascent_03");
    expect(selected()).toEqual([useQueueStore.getState().items[2]?.id]);
  });
});
