import { LucideProvider } from "lucide-react";
import { StrictMode, type ReactNode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { forwardUncaughtErrors } from "./lib/log";
import { followSystemTheme } from "./lib/theme";
import "./styles/globals.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("index.html has no #root element");
}

let root: ReactNode = <App />;

// Outside Tauri (Playwright, `pnpm dev:mock`) the mock backend answers the
// IPC calls, and `/?gallery` shows the component gallery instead of the
// app. The condition is static, so production builds drop both modules.
if (import.meta.env.MODE === "mock") {
  const { installMockBackend } = await import("./ipc/mock");
  window.blurrdMock = installMockBackend();
  if (new URLSearchParams(window.location.search).has("gallery")) {
    const { Gallery } = await import("./gallery/Gallery");
    // The gallery is a long document that scrolls, unlike the app window.
    document.body.style.overflow = "auto";
    root = <Gallery />;
  }
}

forwardUncaughtErrors();
followSystemTheme();

// Icons use a 1.5 px stroke everywhere; components set only their size.
createRoot(container).render(
  <StrictMode>
    <LucideProvider strokeWidth={1.5}>{root}</LucideProvider>
  </StrictMode>,
);
