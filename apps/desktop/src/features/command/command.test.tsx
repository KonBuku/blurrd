import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockBackend } from "@/ipc/mock";
import { useJobSettingsStore } from "@/store/job-settings";
import { useSettingsStore } from "@/store/settings";
import { useUiStore } from "@/store/ui";

import { CommandPalette } from "./CommandPalette";
import { commandScore, paletteCommands, useCommandStore, type PaletteCommand } from "./registry";

function command(
  id: string,
  group: PaletteCommand["group"],
  run = vi.fn<() => void>(),
): PaletteCommand {
  return { id, group, label: id, run };
}

function withQuery(children: ReactNode) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}

/** Opens the palette and returns its dialog. */
async function openPalette(): Promise<HTMLElement> {
  act(() => useCommandStore.getState().setOpen(true));
  return screen.findByRole("dialog", { name: "Command palette" });
}

/** Types into the palette's search field. */
function search(text: string) {
  fireEvent.change(screen.getByRole("combobox", { name: "Search commands" }), {
    target: { value: text },
  });
}

describe("command registry", () => {
  beforeEach(() => useCommandStore.setState({ open: false, sources: {} }));

  it("lists commands in group order, then in registration order", () => {
    const { register } = useCommandStore.getState();
    register("a", [command("help", "Help"), command("add", "File")]);
    register("b", [command("theme", "Theme"), command("open", "File")]);
    expect(paletteCommands(useCommandStore.getState().sources).map((item) => item.id)).toEqual([
      "add",
      "open",
      "theme",
      "help",
    ]);
  });

  it("replaces a source's commands and removes them with an empty list", () => {
    const { register } = useCommandStore.getState();
    register("a", [command("one", "File")]);
    register("a", [command("two", "File")]);
    expect(paletteCommands(useCommandStore.getState().sources).map((item) => item.id)).toEqual([
      "two",
    ]);
    register("a", []);
    expect(useCommandStore.getState().sources).toEqual({});
  });
});

describe("command search", () => {
  it("needs every word and ranks label prefixes, label matches, then keywords", () => {
    expect(commandScore("About blurrd", ["Help"], "")).toBe(1);
    expect(commandScore("About blurrd", ["Help"], "about")).toBe(1);
    expect(commandScore("Theme: Dark", ["Theme", "dark mode"], "theme dark")).toBe(0.8);
    expect(commandScore("Settings: Appearance", ["View"], "appearance")).toBe(0.8);
    expect(commandScore("Theme: Light", ["Theme", "appearance"], "appearance")).toBe(0.5);
    // No fuzzy matches across words.
    expect(commandScore("Round banner: stop blurring names", ["Blur"], "about blurrd")).toBe(0);
    expect(commandScore("Theme: Light", ["Theme", "light mode"], "theme dark")).toBe(0);
  });
});

describe("command palette", () => {
  beforeEach(() => {
    installMockBackend();
    useCommandStore.setState({ open: false, sources: {} });
    useUiStore.setState({ view: "workspace" });
  });

  it("groups the window commands and marks the current theme", async () => {
    render(withQuery(<CommandPalette />));
    const dialog = await openPalette();
    for (const group of ["File", "Blur", "View", "Theme", "Help"]) {
      expect(within(dialog).getByText(group, { selector: "[cmdk-group-heading]" })).toBeTruthy();
    }
    const theme = useSettingsStore.getState().appearance.theme;
    const current = within(dialog).getByRole("option", { name: new RegExp(`Theme: ${theme}`) });
    expect(current.textContent).toContain("Current");
    expect(within(dialog).getByRole("option", { name: /Add recordings/ }).textContent).toContain(
      "Ctrl",
    );
  });

  it("filters by label and keywords and runs the chosen command after closing", async () => {
    render(withQuery(<CommandPalette />));
    await openPalette();
    // Dark is the default (and current) theme, so pick Light.
    search("light mode");
    const options = screen.getAllByRole("option");
    expect(options.map((option) => option.textContent)).toContain("Theme: Light");
    fireEvent.click(screen.getByRole("option", { name: /Theme: Light/ }));
    await waitFor(() => expect(useCommandStore.getState().open).toBe(false));
    await waitFor(() => expect(useSettingsStore.getState().appearance.theme).toBe("Light"));
  });

  it("says so when nothing matches", async () => {
    render(withQuery(<CommandPalette />));
    await openPalette();
    search("zzzz no such command");
    expect(screen.getByText("No matching commands")).toBeTruthy();
    expect(screen.queryAllByRole("option")).toHaveLength(0);
  });

  it("switches a region's blur for the defaults when no file is selected", async () => {
    render(withQuery(<CommandPalette />));
    await openPalette();
    expect(useJobSettingsStore.getState().defaults.values.regions.enabled).toContain("KillFeed");
    fireEvent.click(screen.getByRole("option", { name: /Kill feed: stop blurring names/ }));
    await waitFor(() =>
      expect(useJobSettingsStore.getState().defaults.values.regions.enabled).not.toContain(
        "KillFeed",
      ),
    );
    await openPalette();
    expect(screen.getByRole("option", { name: /Kill feed: blur names/ }).textContent).toContain(
      "Off",
    );
  });

  it("keeps disabled commands listed with the reason and does not run them", async () => {
    const run = vi.fn<() => void>();
    render(withQuery(<CommandPalette />));
    act(() =>
      useCommandStore
        .getState()
        .register("test", [{ ...command("Export", "Export", run), disabledReason: "Nothing" }]),
    );
    await openPalette();
    const option = screen.getByRole("option", { name: /Export/ });
    expect(option.getAttribute("aria-disabled")).toBe("true");
    expect(option.textContent).toContain("Nothing");
    fireEvent.click(option);
    await new Promise((resolve) => window.setTimeout(resolve, 10));
    expect(run).not.toHaveBeenCalled();
  });

  it("still runs the chosen command when it is reopened at once", async () => {
    const run = vi.fn<() => void>();
    render(withQuery(<CommandPalette />));
    act(() => useCommandStore.getState().register("test", [command("Quick", "Help", run)]));
    await openPalette();
    fireEvent.click(screen.getByRole("option", { name: /Quick/ }));
    act(() => useCommandStore.getState().setOpen(true));
    await waitFor(() => expect(run).toHaveBeenCalledTimes(1));
  });

  it("opens the settings page of a command", async () => {
    render(withQuery(<CommandPalette />));
    await openPalette();
    search("shortcuts");
    fireEvent.click(screen.getByRole("option", { name: /Keyboard shortcuts/ }));
    await waitFor(() => expect(useUiStore.getState().view).toBe("settings"));
    expect(useUiStore.getState().settingsSection).toBe("shortcuts");
  });
});
