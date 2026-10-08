import type { MockBackend } from "./ipc/mock";

declare global {
  interface Window {
    /** The mock backend, present only in the `mock` mode (tests, browser dev). */
    blurrdMock?: MockBackend;
  }
}
