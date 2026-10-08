import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";

import { TitleBar } from "./TitleBar";

describe("TitleBar", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
  });

  const commands = () => backend.calls.map((call) => call.cmd);

  it("marks the bar as a drag region but not its buttons", () => {
    const { container } = render(<TitleBar />);
    expect(container.querySelector("header")?.hasAttribute("data-tauri-drag-region")).toBe(true);
    for (const button of screen.getAllByRole("button")) {
      expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
    }
  });

  it("minimizes and closes the window", async () => {
    render(<TitleBar />);
    fireEvent.click(screen.getByRole("button", { name: "Minimize" }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(backend.closed).toBe(true));
    expect(commands()).toContain("plugin:window|minimize");
  });

  it("toggles maximize and follows the window state", async () => {
    render(<TitleBar />);
    fireEvent.click(await screen.findByRole("button", { name: "Maximize" }));
    expect(await screen.findByRole("button", { name: "Restore" })).toBeDefined();
    expect(backend.maximized).toBe(true);

    // A change made outside the button (double-click, snap, Win+Down)
    // arrives as a resize event.
    await act(async () => {
      backend.maximized = false;
      const { emit } = await import("@tauri-apps/api/event");
      await emit("tauri://resize", { width: 1440, height: 900 });
    });
    expect(await screen.findByRole("button", { name: "Maximize" })).toBeDefined();
  });
});
