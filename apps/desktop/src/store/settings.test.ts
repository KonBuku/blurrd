import { beforeEach, describe, expect, it } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { DEFAULT_APPEARANCE } from "@/lib/appearance";

import { useSettingsStore } from "./settings";

/** Writes a settings file the mock backend reads on install. */
function seedSettingsFile(content: Record<string, unknown>) {
  sessionStorage.setItem("blurrd.mock.settings", JSON.stringify(content));
}

/** Waits until queued saves have reached the mock backend. */
async function flushSaves(backend: MockBackend, expected: number) {
  await expect
    .poll(() => backend.calls.filter((call) => call.cmd === "plugin:store|save").length)
    .toBe(expected);
}

describe("settings store", () => {
  let backend: MockBackend;

  beforeEach(() => {
    sessionStorage.clear();
    localStorage.clear();
    document.documentElement.className = "";
    useSettingsStore.setState(useSettingsStore.getInitialState());
  });

  it("loads and applies the appearance from the settings file", async () => {
    seedSettingsFile({
      schema: "blurrd-settings/1",
      version: 1,
      appearance: { theme: "Dark", ui_font_size: 15 },
    });
    backend = installMockBackend();
    await useSettingsStore.getState().load();

    const state = useSettingsStore.getState();
    expect(state.status).toBe("ready");
    expect(state.appearance).toEqual({ ...DEFAULT_APPEARANCE, theme: "Dark", ui_font_size: 15 });
    expect(document.documentElement.classList.contains("dark")).toBe(true);
    expect(document.documentElement.style.getPropertyValue("--ui-font-size")).toBe("15px");
    expect(backend.calls.some((call) => call.cmd === "plugin:store|save")).toBe(false);
  });

  it("saves changes with the schema header and keeps other sections", async () => {
    seedSettingsFile({ processing: { parallel_jobs: 2 } });
    backend = installMockBackend();
    await useSettingsStore.getState().load();

    useSettingsStore.getState().updateAppearance({ theme: "Light" });
    useSettingsStore.getState().updateAppearance({ reduce_motion: "On" });
    await flushSaves(backend, 2);

    expect(backend.settingsFile()).toEqual({
      schema: "blurrd-settings/1",
      version: 1,
      processing: { parallel_jobs: 2 },
      appearance: { ...DEFAULT_APPEARANCE, theme: "Light", reduce_motion: "On" },
    });
    // The next start paints the saved theme before the file has loaded.
    expect(JSON.parse(localStorage.getItem("blurrd.appearance") ?? "{}")).toMatchObject({
      theme: "Light",
    });
  });

  it("never overwrites a settings file of another schema", async () => {
    seedSettingsFile({ schema: "blurrd-settings/2", appearance: { theme: "Dark" } });
    backend = installMockBackend();
    await useSettingsStore.getState().load();
    expect(useSettingsStore.getState().status).toBe("read-only");

    useSettingsStore.getState().updateAppearance({ theme: "Light" });
    expect(document.documentElement.classList.contains("dark")).toBe(false);
    expect(backend.calls.some((call) => call.cmd === "plugin:store|set")).toBe(false);
  });
});
