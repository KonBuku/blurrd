import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { App } from "./App";
import { installMockBackend } from "./ipc/mock";
import { useUiStore } from "./store/ui";

/** Resizes the test window and lets the shell react. */
function resizeWindow(width: number) {
  act(() => {
    window.innerWidth = width;
    window.dispatchEvent(new Event("resize"));
  });
}

describe("App shell", () => {
  beforeEach(() => {
    sessionStorage.clear();
    localStorage.clear();
    installMockBackend();
    useUiStore.setState(useUiStore.getInitialState());
    resizeWindow(1440);
  });

  it("renders the title bar, queue, preview and inspector", () => {
    render(<App />);
    expect(screen.getByRole("banner")).toBeDefined();
    expect(screen.getByRole("complementary", { name: "Queue" })).toBeDefined();
    expect(screen.getByRole("region", { name: "Preview" })).toBeDefined();
    expect(screen.getByRole("region", { name: "Inspector" })).toBeDefined();
    expect(screen.getByRole("tab", { name: "Blur", selected: true })).toBeDefined();
  });

  it("toggles the queue with Ctrl+B and the inspector with Ctrl+Alt+B", () => {
    render(<App />);
    fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true });
    expect(useUiStore.getState().sidebarOpen).toBe(false);
    expect(screen.getByRole("button", { name: "Show queue" })).toBeDefined();

    fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true, altKey: true });
    expect(screen.queryByRole("region", { name: "Inspector" })).toBeNull();
    fireEvent.keyDown(window, { key: "b", code: "KeyB", ctrlKey: true, altKey: true });
    expect(screen.getByRole("region", { name: "Inspector" })).toBeDefined();
  });

  it("resizes the queue with the keyboard within its limits", () => {
    render(<App />);
    const handle = screen.getByRole("separator", { name: "Resize queue" });
    expect(handle.getAttribute("aria-valuenow")).toBe("280");
    fireEvent.keyDown(handle, { key: "ArrowRight" });
    expect(handle.getAttribute("aria-valuenow")).toBe("296");
    fireEvent.keyDown(handle, { key: "Home" });
    expect(handle.getAttribute("aria-valuenow")).toBe("264");
    fireEvent.keyDown(handle, { key: "End" });
    expect(handle.getAttribute("aria-valuenow")).toBe("576");
    fireEvent.doubleClick(handle);
    expect(handle.getAttribute("aria-valuenow")).toBe("280");
  });

  it("shows the inspector as a sheet in narrow windows", async () => {
    resizeWindow(1179);
    render(<App />);
    expect(screen.queryByRole("region", { name: "Inspector" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Show inspector" }));
    expect(await screen.findByRole("dialog", { name: "Inspector" })).toBeDefined();

    // Growing the window closes the sheet and brings the frame back.
    resizeWindow(1180);
    expect(screen.queryByRole("dialog", { name: "Inspector" })).toBeNull();
    expect(screen.getByRole("region", { name: "Inspector" })).toBeDefined();
  });

  it("opens the settings with Ctrl+, and closes them with Escape", async () => {
    render(<App />);
    fireEvent.keyDown(window, { key: ",", code: "Comma", ctrlKey: true });
    expect(screen.getByRole("region", { name: "Settings" })).toBeDefined();
    expect(screen.queryByRole("region", { name: "Preview" })).toBeNull();
    expect(screen.getByRole("button", { name: "Appearance" })).toBe(document.activeElement);
    expect(screen.getByRole("button", { name: "Settings" }).getAttribute("aria-pressed")).toBe(
      "true",
    );

    fireEvent.click(screen.getByRole("button", { name: "Shortcuts" }));
    expect(screen.getByRole("heading", { name: "Shortcuts" })).toBeDefined();

    fireEvent.keyDown(window, { key: "Escape", code: "Escape" });
    expect(screen.queryByRole("region", { name: "Settings" })).toBeNull();
    expect(screen.getByRole("region", { name: "Preview" })).toBeDefined();
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Settings" })).toBe(document.activeElement),
    );
  });
});
