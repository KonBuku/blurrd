import { beforeEach, describe, expect, it, vi } from "vitest";

import { installMockBackend, type MockBackend } from "@/ipc/mock";

import { forwardUncaughtErrors, logWarning } from "./log";

describe("log", () => {
  let backend: MockBackend;

  beforeEach(() => {
    backend = installMockBackend();
  });

  const logged = () =>
    backend.calls
      .filter((call) => call.cmd === "plugin:log|log")
      .map((call) => (call.args as { message: string }).message);

  it("writes warnings to the app log and the console", async () => {
    const consoleWarn = vi.spyOn(console, "warn").mockImplementation(() => {});
    logWarning("opening the window", new Error("denied"));
    await vi.waitFor(() => expect(logged()).toEqual(["opening the window: Error: denied"]));
    expect(consoleWarn).toHaveBeenCalledOnce();
  });

  it("forwards uncaught errors and unhandled rejections", async () => {
    const target = new EventTarget() as Window;
    forwardUncaughtErrors(target);
    target.dispatchEvent(new ErrorEvent("error", { message: "boom" }));
    const rejection = new Event("unhandledrejection") as PromiseRejectionEvent;
    Object.defineProperty(rejection, "reason", { value: "lost" });
    target.dispatchEvent(rejection);
    await vi.waitFor(() =>
      expect(logged()).toEqual(["uncaught error: boom", "unhandled rejection: lost"]),
    );
  });
});
