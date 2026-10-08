import { render, screen } from "@testing-library/react";
import type { ReactNode } from "react";
import { toast } from "sonner";
import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";
import { useQueueStore } from "@/store/queue";

import { dropHint } from "./DropOverlay";
import { browseFiles, browseFolder, importPaths } from "./import";

vi.mock("sonner", () => ({
  toast: {
    info: vi.fn<(message: string) => void>(),
    warning: vi.fn<(message: string, options?: { description?: unknown }) => void>(),
    error: vi.fn<(message: string, options?: { description?: unknown }) => void>(),
  },
}));

/** Renders the description of the last warning toast. */
function lastWarningText(): string {
  const call = vi.mocked(toast.warning).mock.lastCall;
  const options = call?.[1];
  if (!options || typeof options === "function") return "";
  render(<div data-testid="toast">{options.description as ReactNode}</div>);
  return screen.getByTestId("toast").textContent ?? "";
}

function names() {
  return useQueueStore.getState().items.map((item) => item.name);
}

describe("import", () => {
  let backend: MockBackend;

  beforeEach(() => {
    vi.clearAllMocks();
    backend = installMockBackend();
    backend.probeDelayMs = 0;
    useQueueStore.setState(useQueueStore.getInitialState());
  });

  it("adds probed videos and lists unsupported files in one toast", async () => {
    await importPaths([
      "D:\\Recordings\\a.mp4",
      "D:\\Recordings\\cover.png",
      "D:\\Recordings\\corrupt.mp4",
      "D:\\Recordings\\missing.mkv",
    ]);
    expect(names()).toEqual(["a.mp4"]);
    expect(toast.warning).toHaveBeenCalledTimes(1);
    expect(vi.mocked(toast.warning).mock.lastCall?.[0]).toBe("3 files can't be added");
    const text = lastWarningText();
    expect(text).toContain("cover.png — not a supported video");
    expect(text).toContain("corrupt.mp4 — The video stream could not be decoded.");
    expect(text).toContain("missing.mkv — file not found");
  });

  it("lists at most four rejected files by name", async () => {
    await importPaths(["1.txt", "2.txt", "3.txt", "4.txt", "5.txt", "6.txt"]);
    expect(lastWarningText()).toContain("and 2 more");
  });

  it("expands folders and skips what is already queued", async () => {
    await importPaths(["D:\\Recordings\\Ranked"]);
    expect(names()).toEqual(["ascent_ranked_01.mp4", "bind_ranked_02.mp4", "haven_ranked_03.mkv"]);
    expect(toast.warning).not.toHaveBeenCalled();

    // The folder itself is not an item: it is read again, its videos are duplicates.
    await importPaths(["D:\\Recordings\\Ranked"]);
    expect(toast.info).toHaveBeenCalledWith("3 videos were already in the queue");
    await importPaths(["D:\\Recordings\\Ranked\\bind_ranked_02.mp4"]);
    expect(toast.info).toHaveBeenLastCalledWith("Already in the queue");
    await importPaths(["D:\\Recordings\\Ranked\\bind_ranked_02.mp4", "D:\\x.mp4"]);
    expect(names()).toHaveLength(4);
  });

  it("removes the placeholders and reports when the probe fails", async () => {
    backend.probeError = { kind: "Io", message: "The disk is not ready.", detail: null };
    await importPaths(["D:\\Recordings\\Ranked"]);
    expect(names()).toEqual([]);
    expect(toast.error).toHaveBeenCalledWith("The files could not be read", {
      description: "The disk is not ready.",
    });
  });

  it("adds what the dialogs return and nothing when they are canceled", async () => {
    backend.dialogResult = ["D:\\Recordings\\a.mp4"];
    await browseFiles();
    expect(names()).toEqual(["a.mp4"]);

    backend.dialogResult = null;
    await browseFolder();
    expect(names()).toEqual(["a.mp4"]);
    const dialogs = backend.calls.filter((call) => call.cmd === "plugin:dialog|open");
    expect(dialogs).toHaveLength(2);
    expect(dialogs[1]?.args).toMatchObject({ options: { directory: true, multiple: true } });
  });

  it("words the drop hint by count", () => {
    expect(dropHint(0)).toBe("Drop to add recordings");
    expect(dropHint(1)).toBe("Drop to add 1 item");
    expect(dropHint(1200)).toBe("Drop to add 1,200 items");
  });
});
