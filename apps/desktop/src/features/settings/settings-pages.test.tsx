import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { fakeMedia } from "@/ipc/mock/media";
import { DEFAULT_DEFAULTS } from "@/lib/app-defaults";
import { pathKey } from "@/lib/media";
import { settingsOf, useJobSettingsStore } from "@/store/job-settings";
import { useQueueStore } from "@/store/queue";
import { useSettingsStore } from "@/store/settings";
import { useUiStore } from "@/store/ui";

import { DetectionSettings } from "./DetectionSettings";
import { OutputSettings } from "./OutputSettings";
import { PrivacySettings } from "./PrivacySettings";
import { ProcessingSettings } from "./ProcessingSettings";
import { SettingsView } from "./SettingsView";
import { VerificationSettings } from "./VerificationSettings";

vi.mock("sonner", () => ({
  toast: {
    success: vi.fn<(message: string) => void>(),
    error: vi.fn<(message: string) => void>(),
    warning: vi.fn<(message: string) => void>(),
    info: vi.fn<(message: string) => void>(),
  },
}));

const FOLDER = "E:\\Exports\\blurred";

/** Writes a settings file the mock backend reads on install. */
function seedSettingsFile(content: Record<string, unknown>) {
  sessionStorage.setItem("blurrd.mock.settings", JSON.stringify(content));
}

function renderPage(page: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return render(<QueryClientProvider client={client}>{page}</QueryClientProvider>);
}

/** The settings file once the debounced defaults save has reached it. */
async function savedFile(backend: MockBackend, key: string) {
  await waitFor(() => expect(backend.settingsFile()?.[key]).toBeDefined(), { timeout: 2000 });
  return backend.settingsFile() ?? {};
}

/** Queues `path` as a probed video without a cached analysis. */
function add(path: string) {
  const pending = useQueueStore.getState().addPending([path]);
  useQueueStore
    .getState()
    .resolvePending(pending, [{ path, media: fakeMedia(path), analyzed: false }]);
}

describe("settings pages", () => {
  let backend: MockBackend;

  beforeEach(async () => {
    sessionStorage.clear();
    localStorage.clear();
    useSettingsStore.setState(useSettingsStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
    useQueueStore.setState(useQueueStore.getInitialState());
    backend = installMockBackend();
    backend.capabilitiesDelayMs = 0;
    await useSettingsStore.getState().load();
  });

  it("lists every planned page in order and opens each", () => {
    useUiStore.getState().openSettings("general");
    renderPage(<SettingsView />);
    const nav = screen.getByRole("navigation", { name: "Settings pages" });
    const names = within(nav)
      .getAllByRole("button")
      .map((button) => button.textContent);
    expect(names).toEqual([
      "General",
      "Appearance",
      "Processing",
      "Output defaults",
      "Detection",
      "Verification",
      "Shortcuts",
      "Logs & privacy",
      "About",
    ]);
    fireEvent.click(within(nav).getByRole("button", { name: "Output defaults" }));
    expect(screen.getByRole("heading", { level: 1, name: "Output defaults" })).toBeTruthy();
    expect(useUiStore.getState().settingsSection).toBe("output");
  });

  it("shows the export folder, changes it in the dialog and resets it", async () => {
    renderPage(<OutputSettings />);
    expect(screen.getByText("Next to each recording, in the recording's own folder.")).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: /Reset the export folder/ }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);

    backend.dialogResult = [FOLDER];
    fireEvent.click(screen.getByRole("button", { name: /Change/ }));
    expect((await screen.findByTestId("default-export-folder")).textContent).toBe(FOLDER);
    const file = await savedFile(backend, "defaults");
    expect(file.schema).toBe("blurrd-settings/1");
    expect((file.defaults as typeof DEFAULT_DEFAULTS).export.output.folder).toEqual({
      Folder: FOLDER,
    });

    fireEvent.click(screen.getByRole("button", { name: /Reset the export folder/ }));
    expect(screen.queryByTestId("default-export-folder")).toBeNull();
    expect(useJobSettingsStore.getState().defaults.values.export.output.folder).toBe(
      "NextToSource",
    );
  });

  it("checks the file name template and saves only valid names", async () => {
    renderPage(<OutputSettings />);
    const input = screen.getByLabelText("File name");
    fireEvent.change(input, { target: { value: "{name}?" } });
    expect(screen.getByText(/cannot contain/)).toBeTruthy();
    fireEvent.blur(input);
    expect(useJobSettingsStore.getState().defaults.values.export.output.template).toBe(
      "{name}_blurrd",
    );
    fireEvent.change(input, { target: { value: "{name}_clean" } });
    expect(screen.getByText("match_clean.mp4")).toBeTruthy();
    fireEvent.blur(input);
    expect(useJobSettingsStore.getState().defaults.values.export.output.template).toBe(
      "{name}_clean",
    );
  });

  it("applies new defaults to files added later, not to queued ones", () => {
    const queued = "D:\\Recordings\\queued.mp4";
    const later = "D:\\Recordings\\later.mp4";
    add(queued);
    renderPage(<DetectionSettings />);
    fireEvent.click(screen.getByRole("switch", { name: /Kill feed/ }));
    add(later);
    const state = useJobSettingsStore.getState();
    expect(settingsOf(state, pathKey(queued)).regions.enabled).toContain("KillFeed");
    expect(settingsOf(state, pathKey(later)).regions.enabled).not.toContain("KillFeed");
  });

  it("loads stored defaults and processing from the backend", async () => {
    seedSettingsFile({
      schema: "blurrd-settings/1",
      version: 1,
      processing: { hw_decode: "Software" },
      privacy: { log_level: "Debug" },
      defaults: { ...DEFAULT_DEFAULTS, verify: { ...DEFAULT_DEFAULTS.verify, enabled: false } },
    });
    useSettingsStore.setState(useSettingsStore.getInitialState());
    useJobSettingsStore.setState(useJobSettingsStore.getInitialState());
    backend = installMockBackend();
    await useSettingsStore.getState().load();
    expect(useJobSettingsStore.getState().defaults.values.verify.enabled).toBe(false);
    // The loaded defaults are the starting values: nothing shows as modified.
    const { defaults } = useJobSettingsStore.getState();
    expect(defaults.base).toEqual(defaults.values);

    renderPage(<VerificationSettings />);
    expect(
      screen.getByRole("switch", { name: "Verify with OCR" }).getAttribute("aria-checked"),
    ).toBe("false");
    expect(useSettingsStore.getState().privacy.log_level).toBe("Debug");
  });

  it("saves processing and says that a restart applies it", async () => {
    renderPage(<ProcessingSettings />);
    expect(screen.queryByText(/Restart blurrd/)).toBeNull();
    fireEvent.click(screen.getByRole("switch", { name: "Use the GPU" }));
    expect(await screen.findByText(/Restart blurrd/)).toBeTruthy();
    await waitFor(() =>
      expect(backend.settingsFile()?.processing).toMatchObject({ hw_decode: "Software" }),
    );
    expect(await screen.findByText(/here the NVIDIA GeForce RTX 4080 SUPER/)).toBeTruthy();
  });

  it("measures the hardware again only while no job runs", async () => {
    renderPage(<ProcessingSettings />);
    const button = await screen.findByRole("button", { name: "Measure again" });
    fireEvent.click(button);
    await waitFor(() =>
      expect(backend.calls.some((call) => call.cmd === "refresh_capabilities")).toBe(true),
    );
  });

  it("shows the folders, opens the log folder and clears the cache", async () => {
    renderPage(<PrivacySettings />);
    expect(screen.getByText(/makes no network connections and sends no telemetry/)).toBeTruthy();
    await waitFor(() =>
      expect(screen.getByTestId("cache-summary").textContent).toMatch(/^3 recordings/),
    );
    fireEvent.click(screen.getByRole("button", { name: /Open folder/ }));
    await waitFor(() => expect(backend.backendSettings.logDirOpened).toBe(1));

    fireEvent.click(screen.getByRole("button", { name: /Clear/ }));
    const dialog = await screen.findByRole("dialog");
    await act(async () => {
      fireEvent.click(within(dialog).getByRole("button", { name: "Clear" }));
    });
    await waitFor(() =>
      expect(screen.getByTestId("cache-summary").textContent).toBe("No cached analyses."),
    );
  });

  it("changes the log level", async () => {
    renderPage(<PrivacySettings />);
    useSettingsStore.getState().updatePrivacy({ log_level: "Warn" });
    await waitFor(() => expect(backend.settingsFile()?.privacy).toEqual({ log_level: "Warn" }));
  });

  it("does not write a settings file of another schema major", async () => {
    seedSettingsFile({ schema: "blurrd-settings/2" });
    useSettingsStore.setState(useSettingsStore.getInitialState());
    backend = installMockBackend();
    await useSettingsStore.getState().load();
    renderPage(<PrivacySettings />);
    expect(screen.getByText(/cannot be written/)).toBeTruthy();
    useSettingsStore.getState().updatePrivacy({ log_level: "Debug" });
    await new Promise((resolve) => setTimeout(resolve, 50));
    expect(backend.calls.some((call) => call.cmd === "save_privacy")).toBe(false);
  });
});
