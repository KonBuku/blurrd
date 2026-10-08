/**
 * In-memory stand-in for the Tauri backend, used by unit tests, the
 * Playwright end-to-end tests and `pnpm dev:mock` (the frontend in a plain
 * browser). It answers the same IPC commands as the real shell. Production
 * builds never include it: `main.tsx` imports it only in the `mock` mode.
 */
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockIPC, mockWindows } from "@tauri-apps/api/mocks";

/** One IPC call received by the mock backend. */
export interface MockCall {
  readonly cmd: string;
  readonly args: unknown;
}

/** State and call log of an installed mock backend. */
export interface MockBackend {
  /** Every command in the order it arrived (event plugin calls excluded). */
  readonly calls: MockCall[];
  /** Whether the mocked main window is maximized. */
  maximized: boolean;
  /** Whether the mocked main window was closed. */
  closed: boolean;
  /** The settings file as last saved, or `null` before the first save. */
  readonly settingsFile: () => Record<string, unknown> | null;
}

/** Name of the settings file, as opened by the real shell. */
const SETTINGS_FILE = "settings.json";
/** Resource id of the settings store. */
const SETTINGS_RID = 1;
/**
 * Session storage key of the mocked settings file: it survives reloads of
 * the same tab, like the real file survives restarts.
 */
const DISK_KEY = "blurrd.mock.settings";

/** The object members of a store command's arguments. */
interface StoreArgs {
  readonly rid?: number;
  readonly path?: string;
  readonly key?: string;
  readonly value?: unknown;
}

/** Window size reported in mocked resize events, in physical pixels. */
const RESTORED_SIZE = { width: 1440, height: 900 };
const MAXIMIZED_SIZE = { width: 2560, height: 1392 };

/**
 * Installs the mock backend for the window labelled `main` and returns its
 * state. Calling it again replaces the previous backend.
 */
export function installMockBackend(): MockBackend {
  clearMocks();
  mockWindows("main");
  const saved = sessionStorage.getItem(DISK_KEY);
  // The store's in-memory entries; written to session storage on save.
  const settings = new Map<string, unknown>(
    Object.entries(saved === null ? {} : (JSON.parse(saved) as Record<string, unknown>)),
  );
  const backend: MockBackend = {
    calls: [],
    maximized: false,
    closed: false,
    settingsFile: () => {
      const text = sessionStorage.getItem(DISK_KEY);
      return text === null ? null : (JSON.parse(text) as Record<string, unknown>);
    },
  };
  const store = (cmd: string, args: StoreArgs) => {
    if (cmd === "plugin:store|get_store") return args.path === SETTINGS_FILE ? SETTINGS_RID : null;
    if (args.rid !== SETTINGS_RID) throw new Error(`mock backend: unknown store ${args.rid}`);
    switch (cmd) {
      case "plugin:store|get":
        return [settings.get(args.key ?? ""), settings.has(args.key ?? "")];
      case "plugin:store|set":
        settings.set(args.key ?? "", args.value);
        return null;
      case "plugin:store|save":
        sessionStorage.setItem(DISK_KEY, JSON.stringify(Object.fromEntries(settings)));
        return null;
      default:
        throw new Error(`mock backend: unknown command ${cmd}`);
    }
  };

  const resized = () => emit("tauri://resize", backend.maximized ? MAXIMIZED_SIZE : RESTORED_SIZE);

  mockIPC(
    async (cmd, args) => {
      backend.calls.push({ cmd, args });
      switch (cmd) {
        case "plugin:window|is_maximized":
          return backend.maximized;
        case "plugin:window|toggle_maximize":
        case "plugin:window|internal_toggle_maximize":
          backend.maximized = !backend.maximized;
          await resized();
          return null;
        case "plugin:window|close":
          backend.closed = true;
          return null;
        case "plugin:window|minimize":
        case "plugin:window|start_dragging":
        case "plugin:log|log":
          return null;
        default:
          if (cmd.startsWith("plugin:store|")) return store(cmd, args as StoreArgs);
          throw new Error(`mock backend: unknown command ${cmd}`);
      }
    },
    { shouldMockEvents: true },
  );
  return backend;
}
