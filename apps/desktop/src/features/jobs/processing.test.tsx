import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { ExportFooter, ExportShortcut } from "@/features/inspector/ExportFooter";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { startStep, waitingOperation, type OperationProgress } from "@/lib/job-progress";
import { pathKey } from "@/lib/media";
import { useJobSettingsStore } from "@/store/job-settings";
import { useQueueStore } from "@/store/queue";
import { useUiStore } from "@/store/ui";

import { BatchProgress } from "./BatchProgress";
import { FailureDetailsDialog, failureReport, showFailureDetails } from "./FailureDetailsDialog";
import { useJobsStore } from "./jobs";
import { ProcessingCard } from "./ProcessingCard";
import { RecentExports } from "./RecentExports";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn<(message: string) => void>(),
    error: vi.fn<(message: string) => void>(),
    warning: vi.fn<(message: string) => void>(),
    info: vi.fn<(message: string) => void>(),
  },
}));

const PATHS = ["D:\\Recordings\\match_ascent_01.mp4", "D:\\Recordings\\match_bind_02.mp4"];
const ID = pathKey(PATHS[0] ?? "");

/** An export of the first file, rendering frame 5,712 of 9,060. */
function rendering(change: Partial<OperationProgress> = {}): OperationProgress {
  return {
    ...startStep(waitingOperation("export", ["ReverifyRanges", "Render"]), 1),
    stage: "BlurEncode",
    frame: 5712,
    total: 9060,
    fps: 388.2,
    etaMs: 21_000,
    ...change,
  };
}

/** Puts an operation for the first file into the store. */
function running(progress: OperationProgress, job: string | null = "job-7") {
  useJobsStore.setState({
    running: { [ID]: { name: "match_ascent_01.mp4", job, cancelRequested: false, progress } },
    batch: { size: 1, ended: [] },
  });
}

/** Queues the paths as probed videos and selects them all. */
function queueAll() {
  const pending = useQueueStore.getState().addPending(PATHS);
  useQueueStore.getState().resolvePending(
    pending,
    pending.map((path) => ({ path, media: fakeMedia(path), analyzed: false })),
  );
  useQueueStore.getState().selectAll();
}

function withQuery(children: React.ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

describe("processing card", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
    backend.capabilitiesDelayMs = 0;
    useJobsStore.setState(useJobsStore.getInitialState());
    useQueueStore.setState(useQueueStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
    useUiStore.setState({ view: "workspace" });
  });

  it("shows the stages, the overall progress and the figures", () => {
    running(rendering({ warnings: ["One NVENC session"] }));
    render(<ProcessingCard itemId={ID} />);
    const card = screen.getByRole("region", { name: "Processing" });
    expect(within(card).getByRole("heading").textContent).toBe("Exporting match_ascent_01.mp4");
    const steps = within(card)
      .getAllByRole("listitem")
      .map((step) => step.textContent);
    expect(steps).toEqual([
      "Decode(done)",
      "OCR(done)",
      "Verify(done)",
      "Blur + encode",
      "Mux(next)",
    ]);
    expect(within(card).getByRole("listitem", { current: "step" }).textContent).toBe(
      "Blur + encode",
    );
    // The reverification counts as done: (1 + 5712 / 9060) / 2 = 81 %.
    expect(within(card).getByRole("progressbar").getAttribute("aria-valuetext")).toBe("81%");
    expect(screen.getByTestId("processing-figures").textContent).toBe(
      "388 fps · 5,712 / 9,060 frames · 0:21 left · 1 warning",
    );
  });

  it("shows a waiting file with Cancel only", () => {
    running(waitingOperation("export", ["Run"]));
    render(<ProcessingCard itemId={ID} />);
    expect(screen.getByRole("heading").textContent).toBe("Waiting to export match_ascent_01.mp4");
    expect(screen.queryByRole("button", { name: "Pause" })).toBeNull();
    expect(screen.queryByRole("progressbar")).toBeNull();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDefined();
  });

  it("pauses, resumes and cancels through the backend", async () => {
    running(rendering());
    render(<ProcessingCard itemId={ID} />);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Pause" }));
    });
    expect(backend.calls.some((call) => call.cmd === "pause_queue")).toBe(true);
    expect(screen.getByRole("heading").textContent).toBe("Paused · exporting match_ascent_01.mp4");
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Resume" }));
    });
    expect(backend.calls.some((call) => call.cmd === "resume_queue")).toBe(true);
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    });
    expect(backend.calls.find((call) => call.cmd === "cancel_job")?.args).toEqual({ job: "job-7" });
    expect(screen.getByRole("button", { name: "Canceling…" }).hasAttribute("disabled")).toBe(true);
  });

  it("shows nothing without an operation", () => {
    const { container } = render(<ProcessingCard itemId={ID} />);
    expect(container.childElementCount).toBe(0);
  });
});

describe("queue progress, recent exports and failure details", () => {
  beforeEach(() => {
    installMockBackend();
    useJobsStore.setState(useJobsStore.getInitialState());
  });

  it("counts the files of the batch", () => {
    running(rendering({ step: 1, frame: 4530, total: 9060 }));
    useJobsStore.setState({
      batch: {
        size: 2,
        ended: [
          {
            name: "a.mp4",
            operation: "export",
            result: "done",
            text: "verified clean",
            review: false,
            output: "E:\\a_blurrd.mp4",
          },
        ],
      },
    });
    render(<BatchProgress />);
    // (1 + (1 + 0.5) / 2) / 2 = 87 %.
    expect(screen.getByTestId("batch-progress").textContent).toBe("1 of 2 files done87%");
  });

  it("lists recent exports and shows one in Explorer", async () => {
    const backend = installMockBackend();
    useJobsStore.setState({
      recent: [{ path: "E:\\Exports\\a_blurrd.mp4", text: "verified clean" }],
    });
    render(<RecentExports />);
    expect(screen.getByRole("region", { name: "Recent exports" }).textContent).toContain(
      "a_blurrd.mp4verified clean",
    );
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Show a_blurrd.mp4 in folder" }));
    });
    expect(backend.revealed).toEqual(["E:\\Exports\\a_blurrd.mp4"]);
  });

  it("shows and copies the details of a failure", async () => {
    const writeText = vi.fn<(text: string) => Promise<void>>(() => Promise.resolve());
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    const failure = {
      title: "clip.mp4 could not be exported",
      message: "The encoder stopped.",
      detail: "nvenc: out of memory (code 10)",
    };
    render(<FailureDetailsDialog />);
    act(() => showFailureDetails(failure));
    const dialog = await screen.findByRole("dialog", { name: failure.title });
    expect(dialog.textContent).toContain(failure.message);
    expect(
      (within(dialog).getByRole("textbox", { name: "Technical details" }) as HTMLTextAreaElement)
        .value,
    ).toBe(failure.detail);
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Copy details" }));
    });
    expect(writeText).toHaveBeenCalledWith(failureReport(failure));
    expect(failureReport(failure)).toBe(
      "clip.mp4 could not be exported\r\n\r\nThe encoder stopped.\r\n\r\nnvenc: out of memory (code 10)",
    );
    expect(within(dialog).getByRole("button", { name: "Copied" })).toBeDefined();
  });
});

describe("export footer", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
    backend.capabilitiesDelayMs = 0;
    backend.jobs.durationMs = 0;
    useJobsStore.setState(useJobsStore.getInitialState());
    useQueueStore.setState(useQueueStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
    useUiStore.setState({ view: "workspace" });
  });

  it("exports every selected file through the run queue", async () => {
    queueAll();
    render(withQuery(<ExportFooter />));
    const button = await screen.findByRole("button", { name: "Export 2 files" });
    expect(button.getAttribute("aria-keyshortcuts")).toBe("Control+E");
    await act(async () => {
      fireEvent.click(button);
    });
    await waitFor(() => expect(backend.calls.some((call) => call.cmd === "start_run")).toBe(true));
    const run = backend.calls.find((call) => call.cmd === "start_run");
    expect((run?.args as { files?: unknown[] } | undefined)?.files).toHaveLength(2);
    await waitFor(() =>
      expect(useQueueStore.getState().items.every((item) => item.output !== null)).toBe(true),
    );
  });

  it("runs the footer's action with Ctrl+E, also without the inspector", async () => {
    queueAll();
    render(withQuery(<ExportShortcut />));
    await waitFor(() => {
      fireEvent.keyDown(window, { key: "e", code: "KeyE", ctrlKey: true });
      expect(backend.calls.some((call) => call.cmd === "start_run")).toBe(true);
    });
  });
});
