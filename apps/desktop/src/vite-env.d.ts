import type { blurProbe } from "./features/preview/gl/blur-probe";
import type { MockBackend } from "./ipc/mock";

declare global {
  interface Window {
    /** The mock backend, present only in the `mock` mode (tests, browser dev). */
    blurrdMock?: MockBackend;
    /** The WebGL blur test entry point, present only in the `mock` mode. */
    blurrdBlurProbe?: typeof blurProbe;
  }
}
