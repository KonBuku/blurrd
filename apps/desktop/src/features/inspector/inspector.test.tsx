import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { useJobsStore } from "@/features/jobs/jobs";
import type { AnalysisRequest, RenderRequest } from "@/ipc/jobs";
import { MOCK_CAPABILITIES } from "@/ipc/mock/capabilities";
import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { projectId } from "@/ipc/mock/project";
import { analysisRequest, renderRequest } from "@/lib/job-request";
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

/** Picks `label` in the select named `name`. */
async function pick(name: string, label: string) {
  const trigger = screen.getByRole("combobox", { name });
  fireEvent.keyDown(trigger, { key: "Enter" });
  fireEvent.click(await screen.findByRole("option", { name: label }));
  await waitFor(() => expect(trigger.textContent).toBe(label));
}

/** The arguments of the last call of `cmd`, without the channel. */
function lastArgs(backend: MockBackend, cmd: string): Record<string, unknown> {
  const call = backend.calls.findLast((c) => c.cmd === cmd);
  const { onEvent: _channel, ...args } = (call?.args ?? {}) as Record<string, unknown>;
  return args;
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
    backend.capabilitiesDelayMs = 0;
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

  it("round-trips every export setting into the render request", { timeout: 30_000 }, async () => {
    backend.dialogResult = ["E:\\Exports"];
    queue(PATH);
    useUiStore.setState({ inspectorTab: "export" });
    renderInspector();
    await screen.findByTestId("output-preview");
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "HEVC" }).hasAttribute("disabled")).toBe(false),
    );

    choose("Codec", "HEVC");
    await pick("Encoder", "NVENC (GPU)");
    choose("Rate control", "Quality");
    press(screen.getByRole("slider", { name: "Constant quality" }), "ArrowLeft", 3);
    press(screen.getByRole("slider", { name: "Preset" }), "ArrowRight", 2);
    choose("10-bit output", "On");
    choose("NVENC sessions", "2");
    choose("Audio", "AAC");
    await pick("AAC bit rate", "256 kbit/s");
    toggle("Copy subtitles");
    await pick("Container", "MKV");
    choose("Metadata", "All");
    choose("Output folder", "Choose folder");
    await screen.findByText("E:\\Exports");
    const name = screen.getByRole("textbox", { name: "File name" });
    fireEvent.change(name, { target: { value: "{name} clean" } });
    fireEvent.keyDown(name, { key: "Enter" });
    await pick("If the file exists", "Overwrite");
    fireEvent.click(screen.getByRole("button", { name: "Advanced export" }));
    choose("Split encoding", "Off");
    choose("Keyframes", "Encoder default");
    choose("MP4 and MOV index", "End");

    const expected: JobSettings = {
      ...DEFAULT_JOB_SETTINGS,
      export: {
        codec: "Hevc",
        encoder: { Named: "hevc_nvenc" },
        rate: { Cq: 18 },
        preset: 6,
        ten_bit: "On",
        split_encode: "Off",
        nvenc_sessions: "Two",
        keyframes: "EncoderDefault",
        smart_render: false,
        audio: { Aac: 256 },
        copy_subtitles: false,
        metadata: "All",
        container: "Mkv",
        moov: "AtEnd",
        trim: null,
        output: {
          folder: { Folder: "E:\\Exports" },
          template: "{name} clean",
          on_conflict: "Overwrite",
        },
      },
    };
    expect(settingsFor(PATH)).toEqual(expected);
    await waitFor(() =>
      expect(lastArgs(backend, "get_output_preview")).toEqual({
        projectId: projectId(PATH),
        settings: expected.export,
      }),
    );
    await waitFor(() =>
      expect(screen.getByTestId("output-preview").textContent).toContain(
        "match_ascent_01 clean_UNVERIFIED.mkv",
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    await waitFor(() => expect(backend.calls.some((c) => c.cmd === "start_render")).toBe(true));
    const request = lastArgs(backend, "start_render") as unknown as RenderRequest;
    expect(request).toEqual(renderRequest(projectId(PATH), expected));
    expect(request).toEqual({
      projectId: projectId(PATH),
      settings: expected.export,
      blur: expected.blur,
      enabled: expected.regions.enabled,
    });
    await waitFor(() =>
      expect(useQueueStore.getState().items[0]).toMatchObject({
        status: { kind: "unverified" },
        output: "E:\\Exports\\match_ascent_01 clean_UNVERIFIED.mkv",
      }),
    );
  });

  it("disables AV1 with the reason when this PC cannot encode it", async () => {
    backend.capabilities = {
      ...MOCK_CAPABILITIES,
      encoders: MOCK_CAPABILITIES.encoders.filter((encoder) => encoder.codec !== "av1"),
    };
    queue(PATH);
    useUiStore.setState({ inspectorTab: "export" });
    renderInspector();
    await waitFor(() =>
      expect(screen.getByRole("radio", { name: "HEVC" }).hasAttribute("disabled")).toBe(false),
    );
    expect(screen.getByRole("radio", { name: "AV1" }).hasAttribute("disabled")).toBe(true);
    expect(
      screen.getByText(/AV1 is not available: this PC has no working AV1 encoder/),
    ).toBeDefined();
    // H.264 has no 10-bit encoder on this machine: 10-bit cannot be forced on.
    choose("Codec", "H.264");
    expect(screen.getByRole("radio", { name: "On" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("10-bit output needs HEVC or AV1.")).toBeDefined();
  });

  it("keeps the peak bit rate at or above the target and rejects bad file names", async () => {
    queue(PATH);
    useUiStore.setState({ inspectorTab: "export" });
    renderInspector();
    await screen.findByTestId("output-preview");
    choose("Rate control", "Bit rate");
    const target = screen.getByRole("spinbutton", { name: "Target bit rate" });
    fireEvent.change(target, { target: { value: "60" } });
    fireEvent.keyDown(target, { key: "Enter" });
    expect(settingsFor(PATH).export.rate).toEqual({
      Vbr: { target_kbps: 60_000, max_kbps: 60_000 },
    });

    const name = screen.getByRole("textbox", { name: "File name" });
    fireEvent.change(name, { target: { value: "clip?" } });
    fireEvent.keyDown(name, { key: "Enter" });
    expect(name.getAttribute("aria-invalid")).toBe("true");
    expect(screen.getByText(/A file name cannot contain/)).toBeDefined();
    expect(settingsFor(PATH).export.output.template).toBe("{name}_blurrd");
  });

  it("cancels a running export", async () => {
    backend.jobs.durationMs = 2_000;
    queue(PATH);
    useUiStore.setState({ inspectorTab: "export" });
    renderInspector();
    await screen.findByTestId("output-preview");
    fireEvent.click(screen.getByRole("button", { name: "Export" }));
    const cancel = await screen.findByRole("button", { name: "Cancel export" });
    await waitFor(() => expect(cancel.hasAttribute("disabled")).toBe(false));
    fireEvent.click(cancel);
    await waitFor(() =>
      expect(useQueueStore.getState().items[0]?.status).toEqual({ kind: "canceled" }),
    );
    expect(lastArgs(backend, "cancel_job")).toEqual({ job: "job-1" });
  });
});
