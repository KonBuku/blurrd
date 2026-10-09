import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { AppError } from "@/bindings/AppError";
import type { Override } from "@/bindings/Override";
import type { Region } from "@/bindings/Region";
import { getTracks, openProject, setOverrides, type TrackSet } from "@/ipc/commands";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import {
  addBox,
  adjustTrackBox,
  dragRect,
  hideTrackBox,
  MOVE,
  removeEdit,
  setBoxRect,
  setRange,
} from "@/lib/edits";
import { IntervalIndex, overrideBindings } from "@/lib/interval-index";
import { redoEdit, undoEdit, useEditsStore } from "@/store/edits";

import { OverrideSync, SAVE_DEBOUNCE_MS } from "./override-sync";

const PATH = String.raw`D:\Recordings\match_ascent_01.mp4`;
const ALL = new Set<Region>(["KillFeed", "Banner", "VoiceChat", "TextChat", "Manual"]);
const WIDTH = 640;
const HEIGHT = 360;
const FRAMES = 240;

/** A small seeded generator (mulberry32), so a failure can be replayed. */
function rng(seed: number): () => number {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = state;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** The boxes the export would blur on every frame (the plan as rendered). */
function frameTable(set: Pick<TrackSet, "tracks" | "overrides">): string {
  const index = new IntervalIndex(FRAMES, set.tracks, set.overrides);
  return JSON.stringify(
    Array.from({ length: FRAMES }, (_, f) =>
      index.boxesAt(f, ALL).map((b) => [b.rect, b.region, b.seed, b.gridOrigin]),
    ),
  );
}

/**
 * One random edit of `list` (add, move, resize, re-time or delete a
 * manual box, adjust or hide a detector box, delete any edit), or the
 * same list when the drawn edit changes nothing.
 */
function randomEdit(
  list: readonly Override[],
  tracks: TrackSet["tracks"],
  next: () => number,
  uid: () => string,
): readonly Override[] {
  const int = (n: number) => Math.floor(next() * n);
  const adds = list.filter((edit) => edit.op === "Add");
  const pick = <T>(items: readonly T[]) => items[int(items.length)];
  switch (int(6)) {
    case 0: {
      const x = int(WIDTH - 40);
      const y = int(HEIGHT - 20);
      const f0 = int(FRAMES);
      return addBox(
        list,
        uid(),
        [f0, Math.min(FRAMES - 1, f0 + int(90))],
        [x, y, x + 4 + int(36), y + 4 + int(16)],
      );
    }
    case 1: {
      const edit = pick(adds);
      if (edit?.op !== "Add") return list;
      const edges = next() < 0.5 ? MOVE : { left: false, top: true, right: true, bottom: false };
      const rect = dragRect(edit.rect, edges, int(41) - 20, int(21) - 10, WIDTH, HEIGHT);
      return setBoxRect(list, edit.uid, rect);
    }
    case 2: {
      const edit = pick(adds);
      if (!edit) return list;
      const f0 = int(FRAMES);
      return setRange(list, edit.uid, [f0, Math.min(FRAMES - 1, f0 + int(60))]);
    }
    case 3: {
      const edit = pick(list);
      return edit ? removeEdit(list, edit.uid) : list;
    }
    default: {
      const frame = int(FRAMES);
      const index = new IntervalIndex(FRAMES, tracks, list);
      const box = pick(index.boxesAt(frame, ALL).filter((b) => b.trackId !== null));
      if (!box) return list;
      const target = {
        region: box.region,
        span: box.span,
        source: box.source,
        adjustUid: box.adjustUid,
      };
      if (next() < 0.3) return hideTrackBox(list, target, uid());
      const rect = dragRect(box.rect, MOVE, int(7) - 3, int(5) - 2, WIDTH, HEIGHT);
      return adjustTrackBox(list, target, rect, uid());
    }
  }
}

describe("OverrideSync", () => {
  let backend: MockBackend;
  let projectId: string;
  let sync: OverrideSync;
  let errors: AppError[];
  let stale: number;

  beforeEach(async () => {
    sessionStorage.clear();
    backend = installMockBackend();
    projectId = (await openProject(PATH)).id;
    errors = [];
    stale = 0;
    sync = new OverrideSync(projectId, {
      save: setOverrides,
      onSaved: () => {},
      onStale: async () => {
        stale += 1;
        const tracks = await getTracks(projectId);
        sync.adopt(tracks.revision, tracks.overrides);
      },
      onError: (error) => errors.push(error),
    });
    sync.start();
    const tracks = await getTracks(projectId);
    sync.adopt(tracks.revision, tracks.overrides);
  });

  afterEach(() => {
    sync.dispose();
    vi.useRealTimers();
  });

  /** The set_overrides calls the backend received. */
  const saves = () => backend.calls.filter((call) => call.cmd === "set_overrides");

  it("gives back the identical plan after 100 random edits and undo-all", async () => {
    const initial = backend.projects.state(PATH).set;
    const initialTable = frameTable(initial);
    const next = rng(25);
    let counter = 0;
    const uid = () => `00000000-0000-4000-8000-${(counter += 1).toString(16).padStart(12, "0")}`;
    backend.saveDelayMs = 2;

    let edits = 0;
    while (edits < 100) {
      const store = useEditsStore.getState();
      const changed = randomEdit(store.overrides, initial.tracks, next, uid);
      if (changed === store.overrides) continue;
      store.commit(changed);
      edits += 1;
      // Saves overlap with further edits now and then.
      // oxlint-disable-next-line no-await-in-loop -- edits are sequential on purpose
      if (next() < 0.2) await sync.flush();
    }
    await sync.flush();
    const edited = useEditsStore.getState().overrides;
    expect(backend.projects.state(PATH).set.overrides).toEqual(edited);
    expect(frameTable({ tracks: initial.tracks, overrides: edited })).not.toBe(initialTable);
    expect(useEditsStore.temporal.getState().pastStates).toHaveLength(100);

    for (let i = 0; i < 100; i += 1) {
      undoEdit();
      // oxlint-disable-next-line no-await-in-loop -- undo steps are sequential on purpose
      if (next() < 0.2) await sync.flush();
    }
    await sync.flush();

    const stored = backend.projects.state(PATH).set;
    expect(useEditsStore.getState().overrides).toEqual(initial.overrides);
    expect(stored.overrides).toEqual(initial.overrides);
    expect(stored.bindings).toEqual(initial.bindings);
    expect(frameTable(stored)).toBe(initialTable);
    expect(useEditsStore.getState().baseRev).toBe(stored.revision);
    expect(useEditsStore.getState().status).toBe("saved");
    expect(errors).toEqual([]);
    expect(stale).toBe(0);

    // And redo-all brings the edited plan back.
    redoEdit();
    for (let i = 1; i < 100; i += 1) redoEdit();
    await sync.flush();
    expect(backend.projects.state(PATH).set.overrides).toEqual(edited);
  });

  it("sends one save for edits within the debounce, with base_rev", async () => {
    vi.useFakeTimers();
    const base = useEditsStore.getState().baseRev;
    const store = useEditsStore.getState();
    const first = addBox(
      store.overrides,
      "00000000-0000-4000-8000-000000000101",
      [0, 9],
      [1, 1, 9, 9],
    );
    store.commit(first);
    useEditsStore
      .getState()
      .commit(setBoxRect(first, "00000000-0000-4000-8000-000000000101", [2, 2, 9, 9]));
    expect(useEditsStore.getState().status).toBe("pending");
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS - 10);
    expect(saves()).toHaveLength(0);
    await vi.advanceTimersByTimeAsync(20);
    await vi.waitFor(() => expect(useEditsStore.getState().status).toBe("saved"));
    expect(saves()).toHaveLength(1);
    expect(saves()[0]?.args).toMatchObject({ projectId, baseRev: base });
    expect(useEditsStore.getState().baseRev).toBe(base + 1);
  });

  it("sends nothing when undo returns to the stored list before the save", async () => {
    vi.useFakeTimers();
    const store = useEditsStore.getState();
    store.commit(
      addBox(store.overrides, "00000000-0000-4000-8000-000000000102", [0, 9], [1, 1, 9, 9]),
    );
    undoEdit();
    await vi.advanceTimersByTimeAsync(SAVE_DEBOUNCE_MS * 2);
    expect(saves()).toHaveLength(0);
    expect(useEditsStore.getState().status).toBe("saved");
  });

  it("rejects a stale revision and reloads the stored edits", async () => {
    backend.projects.touch(PATH);
    const before = backend.projects.state(PATH).set;
    const store = useEditsStore.getState();
    store.commit(
      addBox(store.overrides, "00000000-0000-4000-8000-000000000103", [0, 9], [1, 1, 9, 9]),
    );
    await sync.flush();

    expect(stale).toBe(1);
    const after = backend.projects.state(PATH).set;
    expect(after.revision).toBe(before.revision);
    expect(after.overrides).toEqual(before.overrides);
    const state = useEditsStore.getState();
    expect(state.overrides).toEqual(before.overrides);
    expect(state.baseRev).toBe(before.revision);
    expect(useEditsStore.temporal.getState().pastStates).toHaveLength(0);
  });

  it("keeps user boxes through a re-analysis and leaves unmatched edits inactive", async () => {
    const tracks = backend.projects.state(PATH).set.tracks;
    const index = new IntervalIndex(FRAMES, tracks, useEditsStore.getState().overrides);
    const box = index.boxesAt(30, ALL).find((b) => b.region === "KillFeed");
    if (!box) throw new Error("fixture has no kill feed box on frame 30");
    const target = { region: box.region, span: box.span, source: box.source, adjustUid: null };
    const store = useEditsStore.getState();
    const moved = adjustTrackBox(
      store.overrides,
      target,
      dragRect(box.rect, MOVE, 0, -2, WIDTH, HEIGHT),
      "00000000-0000-4000-8000-000000000104",
    );
    store.commit(addBox(moved, "00000000-0000-4000-8000-000000000105", [0, 50], [10, 10, 40, 30]));
    await sync.flush();

    backend.projects.reanalyze(PATH, 8);
    // The next save meets the new revision and reloads.
    useEditsStore
      .getState()
      .commit(
        removeEdit(useEditsStore.getState().overrides, "00000000-0000-4000-8000-000000000105"),
      );
    await sync.flush();
    expect(stale).toBe(1);

    const reloaded = useEditsStore.getState().overrides;
    expect(reloaded.map((edit) => edit.uid)).toContain("00000000-0000-4000-8000-000000000105");
    const set = backend.projects.state(PATH).set;
    const bindings = overrideBindings(set.tracks, reloaded);
    expect(bindings.find((b) => b.uid === "00000000-0000-4000-8000-000000000104")?.status).toBe(
      "Unbound",
    );
    expect(bindings.find((b) => b.uid === "00000000-0000-4000-8000-000000000105")?.status).toBe(
      "Bound",
    );
    const after = new IntervalIndex(FRAMES, set.tracks, reloaded).boxesAt(30, ALL);
    expect(after.some((b) => b.adjustUid !== null)).toBe(false);
    expect(after.some((b) => b.uid === "00000000-0000-4000-8000-000000000105")).toBe(true);
  });

  it("marks the frames of changed edits unverified", async () => {
    const store = useEditsStore.getState();
    store.commit(
      addBox(store.overrides, "00000000-0000-4000-8000-000000000106", [40, 60], [1, 1, 9, 9]),
    );
    await sync.flush();
    expect(backend.projects.state(PATH).unverified).toEqual([
      [40, 60],
      [150, 170],
    ]);
  });

  it("keeps the edits and reports other failures", async () => {
    const store = useEditsStore.getState();
    // A box outside the frame: the backend refuses it.
    store.commit(
      addBox(store.overrides, "00000000-0000-4000-8000-000000000107", [0, 9], [600, 1, 700, 9]),
    );
    await sync.flush();
    expect(errors.map((error) => error.kind)).toEqual(["InvalidProject"]);
    expect(useEditsStore.getState().status).toBe("failed");
    expect(useEditsStore.getState().overrides.at(-1)?.uid).toBe(
      "00000000-0000-4000-8000-000000000107",
    );
  });
});
