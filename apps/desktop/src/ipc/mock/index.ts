/**
 * In-memory stand-in for the Tauri backend, used by unit tests, the
 * Playwright end-to-end tests and `pnpm dev:mock` (the frontend in a plain
 * browser). It answers the same IPC commands as the real shell. Production
 * builds never include it: `main.tsx` imports it only in the `mock` mode.
 */
import { emit } from "@tauri-apps/api/event";
import { clearMocks, mockConvertFileSrc, mockIPC, mockWindows } from "@tauri-apps/api/mocks";

import type { AppError } from "@/bindings/AppError";
import type { Capabilities } from "@/bindings/Capabilities";
import type { SessionEntry } from "@/ipc/commands";

import { MOCK_CAPABILITIES } from "./capabilities";

import { createEventBus } from "./events";
import { DEFAULT_FOLDERS, DEFAULT_PICK, probeMedia } from "./media";
import {
  frameCount,
  mockDensity,
  mockProject,
  mockPts,
  mockTracks,
  mockVerifyReport,
  projectId,
} from "./project";
import { installSchemeFetch, type SchemeRequest } from "./scheme";

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
  /** Paths the next file or folder dialog returns; `null` cancels it. */
  dialogResult: string[] | null;
  /** Folders `probe_media` expands: folder path → paths inside it. */
  readonly folders: Map<string, readonly string[]>;
  /** How long `probe_media` takes, in milliseconds. */
  probeDelayMs: number;
  /** When set, `probe_media` rejects with this error. */
  probeError: AppError | null;
  /** Simulates dragging `paths` over the window (Tauri drag-enter event). */
  readonly dragEnter: (paths: readonly string[]) => Promise<void>;
  /** Simulates the drag leaving the window without a drop. */
  readonly dragLeave: () => Promise<void>;
  /** Simulates dragging `paths` onto the window and dropping them. */
  readonly drop: (paths: readonly string[]) => Promise<void>;
  /** Simulates a second app start with `paths` on its command line. */
  readonly secondInstance: (paths: readonly string[]) => Promise<void>;
  /** What `get_capabilities` reports once the background probe is done. */
  capabilities: Capabilities;
  /**
   * How long the background probe runs after the backend was installed,
   * in milliseconds; until then `probe_state` is `Pending`.
   */
  capabilitiesDelayMs: number;
  /** The saved queue as last written, or `null` before the first save. */
  readonly sessionFile: () => SessionEntry[] | null;
  /** Every `blurrd://` request (Exact patches, thumbnails) in order. */
  readonly schemeRequests: SchemeRequest[];
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
/** Session storage key of the mocked `session.json` (the saved queue). */
const SESSION_KEY = "blurrd.mock.session";
/**
 * Session storage key that overrides the probe duration from the first
 * paint on (end-to-end tests set it before the page loads).
 */
const PROBE_DURATION_KEY = "blurrd.mock.capabilitiesDelayMs";
/** Default duration of the mocked background probe, in milliseconds. */
const PROBE_DURATION_MS = 800;

/** The object members of a store command's arguments. */
interface StoreArgs {
  readonly rid?: number;
  readonly path?: string;
  readonly key?: string;
  readonly value?: unknown;
}

/** Pointer position reported in mocked drag events, in physical pixels. */
const DRAG_POSITION = { x: 720, y: 450 };

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
    dialogResult: [...DEFAULT_PICK],
    folders: new Map(DEFAULT_FOLDERS),
    probeDelayMs: 150,
    probeError: null,
    dragEnter: (paths) => emit("tauri://drag-enter", { paths, position: DRAG_POSITION }),
    dragLeave: () => emit("tauri://drag-leave", null),
    drop: async (paths) => {
      await backend.dragEnter(paths);
      await emit("tauri://drag-over", { position: DRAG_POSITION });
      await emit("tauri://drag-drop", { paths, position: DRAG_POSITION });
    },
    secondInstance: (paths) => emit("second-instance", { paths }),
    capabilities: MOCK_CAPABILITIES,
    capabilitiesDelayMs: Number(sessionStorage.getItem(PROBE_DURATION_KEY) ?? PROBE_DURATION_MS),
    sessionFile: () => {
      const text = sessionStorage.getItem(SESSION_KEY);
      return text === null ? null : (JSON.parse(text) as SessionEntry[]);
    },
    schemeRequests: [],
  };
  const installedAt = performance.now();
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

  const events = createEventBus();

  // Opened projects by id, for the commands that take a project id.
  const projects = new Map<string, string>();
  const pathOf = (args: unknown) => {
    const id = (args as { projectId?: string }).projectId ?? "";
    const path = projects.get(id);
    if (path === undefined) throw error("InvalidProject", `mock backend: unknown project ${id}`);
    return path;
  };
  mockConvertFileSrc("windows");
  installSchemeFetch(backend.schemeRequests, (id) => frameCount(projects.get(id) ?? ""));

  mockIPC(async (cmd, args) => {
    if (events.handles(cmd)) return events.invoke(cmd, args as Parameters<typeof events.invoke>[1]);
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
      case "probe_media":
        await delay(backend.probeDelayMs);
        if (backend.probeError) throw backend.probeError;
        return probeMedia((args as { paths: string[] }).paths, backend.folders);
      case "plugin:dialog|open":
        return backend.dialogResult;
      case "get_capabilities":
        return performance.now() - installedAt < backend.capabilitiesDelayMs
          ? { ...backend.capabilities, probe_state: "Pending", encoders: [] }
          : backend.capabilities;
      case "get_session":
        return backend.sessionFile() ?? [];
      case "open_project": {
        const { path } = args as { path: string };
        projects.set(projectId(path), path);
        return mockProject(path);
      }
      case "get_tracks":
        return mockTracks(pathOf(args));
      case "get_pts":
        return mockPts(pathOf(args));
      case "get_timeline_density":
        return mockDensity(pathOf(args), (args as { buckets: number }).buckets);
      case "get_verify_report":
        return mockVerifyReport(pathOf(args));
      case "save_session":
        sessionStorage.setItem(SESSION_KEY, JSON.stringify((args as { entries: unknown }).entries));
        return null;
      case "plugin:window|minimize":
      case "plugin:window|start_dragging":
      case "plugin:log|log":
        return null;
      default:
        if (cmd.startsWith("plugin:store|")) return store(cmd, args as StoreArgs);
        throw new Error(`mock backend: unknown command ${cmd}`);
    }
  });
  return backend;
}

/** An `AppError` without details. */
function error(kind: AppError["kind"], message: string): AppError {
  return { kind, message, detail: null };
}

/** Resolves after `ms` milliseconds (at once for 0). */
function delay(ms: number): Promise<void> {
  return ms <= 0 ? Promise.resolve() : new Promise((resolve) => setTimeout(resolve, ms));
}
