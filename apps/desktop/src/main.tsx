import { LucideProvider } from "lucide-react";
import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { forwardUncaughtErrors } from "./lib/log";
import { followSystemTheme } from "./lib/theme";
import "./styles/globals.css";

const container = document.getElementById("root");
if (!container) {
  throw new Error("index.html has no #root element");
}

// Outside Tauri (Playwright, `pnpm dev:mock`) the mock backend answers the
// IPC calls. The condition is static, so production builds drop the module.
if (import.meta.env.MODE === "mock") {
  const { installMockBackend } = await import("./ipc/mock");
  window.blurrdMock = installMockBackend();
}

forwardUncaughtErrors();
followSystemTheme();

// Icons use a 1.5 px stroke everywhere; components set only their size.
createRoot(container).render(
  <StrictMode>
    <LucideProvider strokeWidth={1.5}>
      <App />
    </LucideProvider>
  </StrictMode>,
);
