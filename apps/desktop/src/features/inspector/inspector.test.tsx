import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useJobsStore } from "@/features/jobs/jobs";
import type { AnalysisRequest } from "@/ipc/jobs";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { projectId } from "@/ipc/mock/project";
import { analysisRequest } from "@/lib/job-request";
import { DEFAULT_JOB_SETTINGS, type JobSettings } from "@/lib/job-settings";
import { pathKey } from "@/lib/media";
import { settingsOf, useJobSettingsStore } from "@/store/job-settings";
import { useQueueStore } from "@/store/queue";
import { useUiStore } from "@/store/ui";

import { InspectorTabs } from "./Inspector";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn<(message: string) => void>(),
    error: vi.fn<(message: string) => void>(),
    warning: vi.fn<(message: string) => void>(),
    info: vi.fn<(message: string) => void>(),
  },
}));

const PATH = "D:\\Recordings\\match_ascent_01.mp4";
const OTHER = "D:\\Recordings\\match_bind_02.mp4";

/** Queues `paths` as probed videos and selects the first. */
function queue(...paths: string[]) {
  const store = useQueueStore.getState();
  const pending = store.addPending(paths);
  useQueueStore.getState().resolvePending(
    pending,
    pending.map((path) => ({ path, media: fakeMedia(path) })),
  );
  useQueueStore.getState().select(pathKey(paths[0] ?? ""), "replace");
}

function renderInspector() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <InspectorTabs />
    </QueryClientProvider>,
  );
}

/** The settings the store holds for a queued path. */
function settingsFor(path: string): JobSettings {
  return settingsOf(useJobSettingsStore.getState(), pathKey(path));
}

/** Toggles the switch whose name starts with `name` (region names carry a count). */
function toggle(name: string) {
  fireEvent.click(screen.getByRole("switch", { name: new RegExp(`^${name}`) }));
}

/** Clicks the segment `label` of the toggle group named `group`. */
function choose(group: string, label: string) {
  fireEvent.click(
    within(screen.getByRole("radiogroup", { name: group })).getByRole("radio", { name: label }),
  );
}

/** Presses `key` `times` times on the element. */
function press(element: HTMLElement, key: string, times = 1) {
  for (let i = 0; i < times; i += 1) fireEvent.keyDown(element, { key });
}

describe("inspector", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
    backend.jobs.durationMs = 0;
    useQueueStore.setState(useQueueStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
    useJobsStore.setState({ running: {} });
    useUiStore.setState({ inspectorTab: "blur" });
  });

  it(
    "round-trips every blur and detection setting into the analysis request",
    { timeout: 20_000 },
    async () => {
      queue(PATH);
      renderInspector();
      await screen.findByText("Manual boxes");

      // Regions: one HUD region and one screen surface off, the options of
      // the kill feed and the text chat changed.
      toggle("Voice chat");
      toggle("Scoreboard");
      toggle("Keep my own row readable");
      const names = screen.getByRole("textbox", { name: "Names to keep readable" });
      fireEvent.change(names, { target: { value: "Ally, Friend , Ally" } });
      fireEvent.keyDown(names, { key: "Enter" });
      toggle("Also blur message text");
      toggle("Blur names in system lines");

      // Style.
      choose("Strength", "Custom");
      const block = screen.getByRole("spinbutton", { name: "Custom block size" });
      press(block, "ArrowUp", 2);
      press(screen.getByRole("spinbutton", { name: "Horizontal padding" }), "ArrowUp", 2);
      press(screen.getByRole("spinbutton", { name: "Vertical padding" }), "ArrowDown");
      press(screen.getByRole("slider", { name: "Corners" }), "ArrowRight", 2);
      press(screen.getByRole("slider", { name: "Feather" }), "ArrowLeft");
      toggle("Noise");

      // Advanced detection.
      fireEvent.click(screen.getByRole("button", { name: "Advanced detection" }));
      choose("Box fit", "Safe");
      choose("Sensitivity", "High");
      press(screen.getByRole("spinbutton", { name: "Start early" }), "ArrowUp");
      press(screen.getByRole("spinbutton", { name: "Stay at least" }), "ArrowDown", 3);
      press(screen.getByRole("spinbutton", { name: "Bridge gaps up to" }), "ArrowUp", 5);

      const expected: JobSettings = {
        ...DEFAULT_JOB_SETTINGS,
        regions: {
          enabled: ["KillFeed", "Banner", "TextChat", "DeathRecap", "Spectating", "MatchEnd"],
          mode: "Safe",
          keep_names: { own_row: true, names: ["Ally", "Friend"] },
          chat_blur_message: true,
          chat_blur_system: false,
        },
        detector: {
          sensitivity: "High",
          temporal: { pre_roll: 4, post_roll_min: 15, max_gap: 50 },
        },
        blur: {
          style: "PixelBlur",
          strength: { Custom: 12 },
          pad: [6, 2],
          radius: 8,
          feather: 1,
          noise: false,
        },
      };
      expect(settingsFor(PATH)).toEqual(expected);

      fireEvent.click(screen.getByRole("button", { name: "Re-analyze" }));
      await waitFor(() => expect(backend.calls.some((c) => c.cmd === "start_analysis")).toBe(true));
      const call = backend.calls.find((c) => c.cmd === "start_analysis");
      const { onEvent, ...request } = (call?.args ?? {}) as AnalysisRequest & { onEvent: unknown };
      expect(onEvent).toBeDefined();
      expect(request).toEqual(analysisRequest(projectId(PATH), expected));
      expect(request).toEqual({
        projectId: projectId(PATH),
        regions: expected.regions,
        detector: expected.detector,
        verify: expected.verify,
      });
      await waitFor(() =>
        expect(useQueueStore.getState().items[0]?.status).toEqual({ kind: "ready", boxes: null }),
      );
    },
  );

  it("keeps per-file values and applies a change to every file with All", async () => {
    queue(PATH, OTHER);
    renderInspector();
    await screen.findByText("Manual boxes");
    toggle("Kill feed");
    expect(settingsFor(PATH).regions.enabled).not.toContain("KillFeed");
    expect(settingsFor(OTHER).regions.enabled).toContain("KillFeed");
    expect(useJobSettingsStore.getState().defaults).toBe(DEFAULT_JOB_SETTINGS);

    choose("Apply changes to", "All");
    expect(screen.getByTestId("inspector-target").textContent).toBe("All 2 files and new files");
    toggle("Text chat");
    for (const settings of [
      settingsFor(PATH),
      settingsFor(OTHER),
      useJobSettingsStore.getState().defaults,
    ]) {
      expect(settings.regions.enabled).not.toContain("TextChat");
    }
    // Only the changed field moves: the other file keeps its kill feed.
    expect(settingsFor(OTHER).regions.enabled).toContain("KillFeed");
    expect(settingsFor(PATH).regions.enabled).not.toContain("KillFeed");
  });

  it("edits the defaults for new files when nothing is selected", () => {
    renderInspector();
    expect(screen.getByTestId("inspector-target").textContent).toBe("Defaults for new files");
    fireEvent.click(screen.getByRole("checkbox", { name: "All regions" }));
    expect(useJobSettingsStore.getState().defaults.regions.enabled).toEqual([]);
    fireEvent.click(screen.getByRole("checkbox", { name: "All regions" }));
    expect(useJobSettingsStore.getState().defaults.regions.enabled).toHaveLength(8);
    queue(PATH);
    expect(settingsFor(PATH)).toBe(useJobSettingsStore.getState().defaults);
  });

  it("names both region groups and says world name tags are out of scope", () => {
    renderInspector();
    expect(screen.getByRole("heading", { name: "HUD" })).toBeDefined();
    expect(screen.getByRole("heading", { name: "Screens" })).toBeDefined();
    const all = screen.getByRole("checkbox", { name: "All regions" });
    expect(all.getAttribute("aria-describedby")).toBeTruthy();
    const note = document.getElementById(all.getAttribute("aria-describedby") ?? "");
    expect(note?.textContent).toMatch(/name tags .* out of scope/i);
    toggle("Death recap");
    expect(all.getAttribute("data-state")).toBe("indeterminate");
  });

  it("marks the file failed when the analysis fails", async () => {
    queue(PATH);
    renderInspector();
    await screen.findByText("Manual boxes");
    backend.jobs.failNext = {
      kind: "Decode",
      message: "The video could not be decoded.",
      detail: null,
    };
    fireEvent.click(screen.getByRole("button", { name: "Advanced detection" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Re-analyze" }));
    });
    await waitFor(() =>
      expect(useQueueStore.getState().items[0]?.status).toEqual({
        kind: "failed",
        reason: "The video could not be decoded.",
        detail: null,
      }),
    );
  });
});
