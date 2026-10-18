import { QueryClient } from "@tanstack/react-query";
import { renderHook } from "@testing-library/react";
import { toast } from "sonner";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { Stage } from "@/bindings/Stage";
import { openProject } from "@/ipc/commands";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { DEFAULT_GENERAL } from "@/lib/general";
import { JOB_STAGES } from "@/lib/job-stages";
import { DEFAULT_JOB_SETTINGS } from "@/lib/job-settings";
import { pathKey } from "@/lib/media";
import { useQueueStore, type QueueItem } from "@/store/queue";
import { useSettingsStore } from "@/store/settings";

import { useFailureDetailsStore } from "./FailureDetailsDialog";
import {
  cancelItem,
  exportItem,
  exportItems,
  setPaused,
  useJobsStore,
  type RunEntry,
} from "./jobs";
import { jobsTaskbarState, useTaskbarProgress } from "./useTaskbarProgress";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn<(message: string) => void>(),
    error: vi.fn<(message: string) => void>(),
    warning: vi.fn<(message: string) => void>(),
    info: vi.fn<(message: string) => void>(),
  },
}));

const PATHS = [
  "D:\\Recordings\\match_ascent_01.mp4",
  "D:\\Recordings\\match_bind_02.mp4",
  "D:\\Recordings\\fresh_lotus_03.mp4",
];

/** Queues `paths` as probed videos and returns their items. */
function queue(...paths: string[]): QueueItem[] {
  const pending = useQueueStore.getState().addPending(paths);
  useQueueStore.getState().resolvePending(
    pending,
    pending.map((path) => ({ path, media: fakeMedia(path), analyzed: false })),
  );
  return useQueueStore.getState().items.filter((queued) => paths.includes(queued.path));
}

/** The current queue item of `path`. */
function item(path: string): QueueItem {
  const found = useQueueStore.getState().items.find((candidate) => candidate.id === pathKey(path));
  if (!found) throw new Error(`${path} is not queued`);
  return found;
}

/** Resolves once `check` holds, polling every few milliseconds. */
async function until(check: () => boolean, timeoutMs = 5000): Promise<void> {
  const start = performance.now();
  while (!check()) {
    if (performance.now() - start > timeoutMs) throw new Error("timed out");
    // oxlint-disable-next-line no-await-in-loop -- polling on purpose
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
}

/** Opens the projects of `items` and builds run entries with the default settings. */
async function entries(items: readonly QueueItem[]): Promise<RunEntry[]> {
  return Promise.all(
    items.map(async (queued) => ({
      item: queued,
      projectId: (await openProject(queued.path)).id,
      settings: DEFAULT_JOB_SETTINGS,
    })),
  );
}

/** Commands the mock received, without the event plugin and the log. */
function commands(backend: MockBackend): string[] {
  return backend.calls.map((call) => call.cmd).filter((cmd) => !cmd.startsWith("plugin:log"));
}

const idle = () => Object.keys(useJobsStore.getState().running).length === 0;

describe("jobs", () => {
  let backend: MockBackend;
  let client: QueryClient;
  let unmount: () => void;

  beforeEach(() => {
    backend = installMockBackend();
    backend.jobs.durationMs = 0;
    client = new QueryClient();
    useQueueStore.setState(useQueueStore.getInitialState());
    useJobsStore.setState(useJobsStore.getInitialState());
    useFailureDetailsStore.setState({ shown: null });
    useSettingsStore.setState({ general: { ...DEFAULT_GENERAL, notify: true } });
    vi.mocked(toast.success).mockClear();
    vi.mocked(toast.error).mockClear();
    vi.mocked(toast.warning).mockClear();
    ({ unmount } = renderHook(() => useTaskbarProgress()));
  });

  afterEach(() => unmount());

  it("verifies edited ranges before it renders, then rests the file", async () => {
    const [file] = queue(PATHS[0] ?? "");
    if (!file) throw new Error("not queued");
    const project = await openProject(file.path);
    await exportItem(file, { id: project.id, unverified: true }, DEFAULT_JOB_SETTINGS, client);
    await until(idle);
    const jobs = commands(backend).filter((cmd) => cmd.startsWith("start_"));
    expect(jobs).toEqual(["start_reverify", "start_render"]);
    expect(backend.projects.state(file.path).unverified).toEqual([]);
    expect(item(file.path).output).toMatch(/match_ascent_01_blurrd/);
    expect(useJobsStore.getState().recent[0]?.path).toBe(item(file.path).output);
  });

  it("renders at once when nothing is unverified or verification is off", async () => {
    const [file] = queue(PATHS[0] ?? "");
    if (!file) throw new Error("not queued");
    const project = await openProject(file.path);
    const noVerify = {
      ...DEFAULT_JOB_SETTINGS,
      verify: { ...DEFAULT_JOB_SETTINGS.verify, enabled: false },
    };
    await exportItem(file, { id: project.id, unverified: true }, noVerify, client);
    await until(idle);
    expect(commands(backend).filter((cmd) => cmd.startsWith("start_"))).toEqual(["start_render"]);
  });

  it.each(JOB_STAGES.Run.map((stage) => [stage]))(
    "cancels a run job during %s",
    async (stage: Stage) => {
      backend.jobs.durationMs = 700;
      // A file whose analysis is not current runs the whole pipeline.
      const files = queue(PATHS[2] ?? "");
      await exportItems(await entries(files), client);
      const id = files[0]?.id ?? "";
      await until(() => useJobsStore.getState().running[id]?.progress.stage === stage);
      await cancelItem(id);
      await until(idle);
      expect(item(PATHS[2] ?? "").status).toEqual({ kind: "canceled" });
      expect(jobsTaskbarState(useJobsStore.getState())).toEqual({ status: "none", progress: 0 });
      expect(backend.outputs.size).toBe(0);
    },
  );

  it("cancels before the backend has answered the job id", async () => {
    backend.jobs.durationMs = 200;
    const [file] = queue(PATHS[0] ?? "");
    if (!file) throw new Error("not queued");
    const project = await openProject(file.path);
    const started = exportItem(
      file,
      { id: project.id, unverified: true },
      DEFAULT_JOB_SETTINGS,
      client,
    );
    expect(useJobsStore.getState().running[file.id]?.job).toBeNull();
    await cancelItem(file.id);
    await started;
    await until(idle);
    expect(item(file.path).status).toEqual({ kind: "canceled" });
    expect(commands(backend)).toContain("cancel_job");
    expect(commands(backend)).not.toContain("start_render");
  });

  it("shows the failure message and its details", async () => {
    backend.jobs.failNext = {
      kind: "Encode",
      message: "The encoder stopped.",
      detail: "nvenc: out of memory (code 10)",
    };
    const [file] = queue(PATHS[0] ?? "");
    if (!file) throw new Error("not queued");
    const project = await openProject(file.path);
    await exportItem(file, { id: project.id, unverified: false }, DEFAULT_JOB_SETTINGS, client);
    await until(idle);
    expect(item(file.path).status).toEqual({
      kind: "failed",
      reason: "The encoder stopped.",
      detail: "nvenc: out of memory (code 10)",
    });
    const [title, options] = vi.mocked(toast.error).mock.calls.at(-1) ?? [];
    expect(title).toBe("match_ascent_01.mp4 could not be exported");
    const action = (options as { action?: { label: string; onClick: () => void } }).action;
    expect(action?.label).toBe("Details");
    action?.onClick();
    expect(useFailureDetailsStore.getState().shown).toEqual({
      title: "match_ascent_01.mp4 could not be exported",
      message: "The encoder stopped.",
      detail: "nvenc: out of memory (code 10)",
    });
  });

  it("runs several files one after another through the run queue", async () => {
    backend.jobs.durationMs = 60;
    const files = queue(...PATHS);
    await exportItems(await entries(files), client);
    // The first file runs, the others wait.
    await until(() => item(PATHS[0] ?? "").status.kind === "exporting");
    expect(item(PATHS[1] ?? "").status.kind).toBe("waiting");
    expect(item(PATHS[2] ?? "").status.kind).toBe("waiting");
    const run = backend.calls.find((call) => call.cmd === "start_run");
    expect((run?.args as { files?: unknown[] } | undefined)?.files).toHaveLength(3);

    await until(idle);
    for (const path of PATHS) expect(item(path).output).not.toBeNull();
    // The taskbar only moves forwards, then clears.
    const percents = backend.taskbar
      .filter((call) => call.status === "normal")
      .map((call) => call.progress ?? 0);
    expect(percents.length).toBeGreaterThan(3);
    expect(percents).toEqual(percents.toSorted((a, b) => a - b));
    expect(backend.taskbar.at(-1)).toEqual({ status: "none", progress: 0 });
  });

  it("cancels a file that waits in the run queue without touching the others", async () => {
    backend.jobs.durationMs = 120;
    const files = queue(...PATHS);
    await exportItems(await entries(files), client);
    const waiting = files[2]?.id ?? "";
    await until(() => useJobsStore.getState().running[waiting]?.job !== null);
    await cancelItem(waiting);
    await until(() => item(PATHS[2] ?? "").status.kind === "canceled");
    expect(useJobsStore.getState().running[files[0]?.id ?? ""]).toBeDefined();
    await until(idle);
    expect(item(PATHS[0] ?? "").status.kind).not.toBe("canceled");
    expect(item(PATHS[2] ?? "").output).toBeNull();
  });

  it("pauses and resumes the jobs", async () => {
    backend.jobs.durationMs = 200;
    const files = queue(PATHS[0] ?? "");
    await exportItems(await entries(files), client);
    await until(
      () => (useJobsStore.getState().running[files[0]?.id ?? ""]?.progress.frame ?? 0) > 0,
    );
    await setPaused(true);
    expect(jobsTaskbarState(useJobsStore.getState()).status).toBe("paused");
    const frame = useJobsStore.getState().running[files[0]?.id ?? ""]?.progress.frame;
    await new Promise((resolve) => setTimeout(resolve, 150));
    expect(useJobsStore.getState().running[files[0]?.id ?? ""]?.progress.frame).toBe(frame);
    await setPaused(false);
    await until(idle);
    expect(commands(backend)).toEqual(expect.arrayContaining(["pause_queue", "resume_queue"]));
    expect(item(PATHS[0] ?? "").output).not.toBeNull();
  });

  it("notifies once per batch, only while the window is in the background", async () => {
    const files = queue(PATHS[0] ?? "", PATHS[1] ?? "");
    await exportItems(await entries(files), client);
    await until(idle);
    await until(() => commands(backend).includes("plugin:window|is_focused"));
    expect(backend.notifications).toEqual([]);

    backend.focused = false;
    await exportItems(await entries(files.map((file) => item(file.path))), client);
    await until(idle);
    await until(() => backend.notifications.length > 0);
    expect(backend.notifications).toEqual([
      { title: "Export finished", body: "2 exported · 2 need review" },
    ]);

    useSettingsStore.setState({ general: { ...DEFAULT_GENERAL, notify: false } });
    await exportItems(await entries(files.map((file) => item(file.path))), client);
    await until(idle);
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(backend.notifications).toHaveLength(1);
  });

  it("opens the folder of the last export when the settings say so", async () => {
    useSettingsStore.setState({ general: { ...DEFAULT_GENERAL, open_folder_after_export: true } });
    const files = queue(PATHS[0] ?? "", PATHS[1] ?? "");
    await exportItems(await entries(files), client);
    await until(idle);
    await until(() => backend.revealed.length > 0);
    expect(backend.revealed).toEqual([item(PATHS[1] ?? "").output]);
  });

  it("delivers events that arrive before the job ids", async () => {
    backend.runAnswerDelayMs = 50;
    const files = queue(PATHS[0] ?? "");
    await exportItems(await entries(files), client);
    await until(idle);
    expect(item(PATHS[0] ?? "").status.kind).not.toBe("waiting");
    expect(item(PATHS[0] ?? "").output).not.toBeNull();
  });
});
