import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";

import { TitleBar } from "./TitleBar";

/** The title bar with the queue sidebar open. */
function titleBar() {
  return <TitleBar sidebarOpen onToggleSidebar={() => undefined} />;
}

describe("TitleBar", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
  });

  const commands = () => backend.calls.map((call) => call.cmd);

  it("marks the bar as a drag region but not its buttons", () => {
    const { container } = render(titleBar());
    expect(container.querySelector("header")?.hasAttribute("data-tauri-drag-region")).toBe(true);
    for (const button of screen.getAllByRole("button")) {
      expect(button.hasAttribute("data-tauri-drag-region")).toBe(false);
    }
  });

  it("minimizes and closes the window", async () => {
    render(titleBar());
    fireEvent.click(screen.getByRole("button", { name: "Minimize" }));
    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    await waitFor(() => expect(backend.closed).toBe(true));
    expect(commands()).toContain("plugin:window|minimize");
  });

  it("toggles maximize and follows the window state", async () => {
    render(titleBar());
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

  it("toggles the queue sidebar from the app mark", () => {
    const onToggle = vi.fn<() => void>();
    const { rerender } = render(<TitleBar sidebarOpen onToggleSidebar={onToggle} />);
    const hide = screen.getByRole("button", { name: "Hide queue" });
    expect(hide.getAttribute("aria-expanded")).toBe("true");
    fireEvent.click(hide);
    expect(onToggle).toHaveBeenCalledOnce();

    rerender(<TitleBar sidebarOpen={false} onToggleSidebar={onToggle} />);
    expect(screen.getByRole("button", { name: "Show queue" }).getAttribute("aria-expanded")).toBe(
      "false",
    );
  });
});
