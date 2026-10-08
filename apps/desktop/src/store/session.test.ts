import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { SessionEntry } from "@/ipc/commands";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { DEFAULT_GENERAL } from "@/lib/general";

import { useQueueStore } from "./queue";
import { parseSession, SAVE_DELAY_MS, sessionEntries, startSession } from "./session";
import { useSettingsStore } from "./settings";

/** Seeds the mocked session file as a previous run would have left it. */
function seedSession(entries: readonly SessionEntry[]) {
  sessionStorage.setItem("blurrd.mock.session", JSON.stringify(entries));
}

describe("session", () => {
  let backend: MockBackend;
  let stop: (() => void) | undefined;

  beforeEach(() => {
    sessionStorage.clear();
    backend = installMockBackend();
    backend.probeDelayMs = 0;
    useQueueStore.setState(useQueueStore.getInitialState());
    useSettingsStore.setState({ general: DEFAULT_GENERAL });
  });

  afterEach(() => {
    stop?.();
    stop = undefined;
    vi.useRealTimers();
  });

  it("restores the saved queue with its statuses and reads the files again", async () => {
    seedSession([
      {
        path: "D:\\Recordings\\done.mp4",
        status: { kind: "done", verified: true },
        output: "D:\\Out\\done_blurrd.mp4",
      },
      { path: "D:\\Recordings\\review.mp4", status: { kind: "unverified" }, output: null },
      { path: "D:\\Recordings\\missing.mp4", status: { kind: "queued" }, output: null },
    ]);
    stop = await startSession();

    const items = useQueueStore.getState().items;
    expect(items.map((item) => item.name)).toEqual(["done.mp4", "review.mp4", "missing.mp4"]);
    expect(items[0]?.status).toEqual({ kind: "done", verified: true });
    expect(items[0]?.output).toBe("D:\\Out\\done_blurrd.mp4");
    expect(items[0]?.media).toEqual(fakeMedia("D:\\Recordings\\done.mp4"));
    expect(items[1]?.status).toEqual({ kind: "unverified" });
    expect(items[2]?.status).toMatchObject({ kind: "failed", reason: "File not found" });
  });

  it("saves changes after a short delay, leaving placeholders out", async () => {
    vi.useFakeTimers();
    stop = await startSession();
    const queue = useQueueStore.getState();
    const pending = queue.addPending(["D:\\a.mp4", "D:\\b.mp4"]);
    useQueueStore
      .getState()
      .resolvePending(pending.slice(0, 1), [{ path: "D:\\a.mp4", media: fakeMedia("D:\\a.mp4") }]);

    expect(backend.sessionFile()).toBeNull();
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(backend.sessionFile()).toEqual([
      { path: "D:\\a.mp4", status: { kind: "queued" }, output: null },
    ]);
  });

  it("keeps no paths when restoring is off", async () => {
    vi.useFakeTimers();
    seedSession([{ path: "D:\\old.mp4", status: { kind: "queued" }, output: null }]);
    useSettingsStore.setState({ general: { ...DEFAULT_GENERAL, restore_queue: false } });
    stop = await startSession();
    expect(useQueueStore.getState().items).toEqual([]);
    expect(backend.sessionFile()).toEqual([]);

    useQueueStore.getState().addPending(["D:\\x.mp4"]);
    useQueueStore
      .getState()
      .resolvePending(["D:\\x.mp4"], [{ path: "D:\\x.mp4", media: fakeMedia("D:\\x.mp4") }]);
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(backend.sessionFile()).toEqual([]);

    // Turning it on saves the current queue.
    useSettingsStore.setState({ general: DEFAULT_GENERAL });
    await vi.advanceTimersByTimeAsync(SAVE_DELAY_MS);
    expect(backend.sessionFile()).toHaveLength(1);
  });

  it("saves running jobs as resting statuses", () => {
    const pending = useQueueStore.getState().addPending(["D:\\a.mp4"]);
    useQueueStore
      .getState()
      .resolvePending(pending, [{ path: "D:\\a.mp4", media: fakeMedia("D:\\a.mp4") }]);
    const [item] = useQueueStore.getState().items;
    if (!item) throw new Error("no item");
    const running = {
      ...item,
      status: { kind: "exporting", progress: 0.5, fps: 300, etaMs: 1000 },
    } as const;
    expect(sessionEntries([running])).toEqual([
      { path: "D:\\a.mp4", status: { kind: "ready", boxes: null }, output: null },
    ]);
  });

  it("drops malformed entries of the session file", () => {
    expect(parseSession({})).toEqual([]);
    expect(
      parseSession([
        null,
        { path: "" },
        { path: 3 },
        { path: "D:\\a.mp4", status: { kind: "nope" }, output: 1 },
      ]),
    ).toEqual([{ path: "D:\\a.mp4", status: { kind: "queued" }, output: null }]);
  });
});
