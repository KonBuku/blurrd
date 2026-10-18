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
import type { Override } from "@/bindings/Override";
import type { Preset } from "@/bindings/Preset";
import type { SessionEntry } from "@/ipc/commands";
import type { AnalysisRequest, RenderRequest, RunFile } from "@/ipc/jobs";
import type { ExportSettings } from "@/bindings/ExportSettings";
import { pathKey } from "@/lib/media";

import { MOCK_CAPABILITIES } from "./capabilities";

import { MockProjects } from "./edits";
import { createEventBus } from "./events";
import { MockJobs, type MockChannel } from "./jobs";
import { installNotificationShim } from "./notification";
import { mockOutput } from "./output";
import { MockPresets } from "./presets";
import { DEFAULT_FOLDERS, DEFAULT_PICK, probeMedia } from "./media";
import { frameCount, mockDensity, mockProject, mockPts, projectId } from "./project";
import { installSchemeFetch, type SchemeRequest } from "./scheme";
import { MockSettings } from "./settings";

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
  /**
   * Paths the next file or folder dialog returns (the first one for a
   * single-choice dialog); `null` cancels it.
   */
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
  /**
   * The stored projects (tracks, edits, revision, unverified ranges); tests
   * read them and simulate other writers and re-analyses with them.
   */
  readonly projects: MockProjects;
  /** How long `set_overrides` takes, in milliseconds. */
  saveDelayMs: number;
  /** The jobs (analysis, export), their duration and a failure to inject. */
  readonly jobs: MockJobs;
  /** Files written by exports (`pathKey`s), for the conflict policy. */
  readonly outputs: Set<string>;
  /** Presets and the preset files of the fake file system. */
  readonly presets: MockPresets;
  /**
   * How long `start_run` waits before it answers the job ids, in
   * milliseconds; the jobs already run, so their first events arrive
   * before the ids (a race the frontend must handle).
   */
  runAnswerDelayMs: number;
  /** Whether the mocked main window has the focus (`is_focused`). */
  focused: boolean;
  /** Every taskbar progress state the window was given, in order. */
  readonly taskbar: TaskbarCall[];
  /** Every system notification shown, in order. */
  readonly notifications: { readonly title: string; readonly body: string }[];
  /** Every file shown in Explorer, in order. */
  readonly revealed: string[];
  /** URLs opened in the browser (opener plugin), in order. */
  readonly opened: string[];
  /**
   * The backend's settings sections (processing, defaults, privacy), the
   * cache figures and how often the log folder was opened.
   */
  readonly backendSettings: MockSettings;
}

/** The value of one `set_progress_bar` call. */
export interface TaskbarCall {
  readonly status?: string;
  readonly progress?: number;
}

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

/** A file of the project cache of `path` (`%LOCALAPPDATA%\blurrd\projects`). */
function cacheFile(path: string, name: string): string {
  return [
    "C:",
    "Users",
    "me",
    "AppData",
    "Local",
    "blurrd",
    "projects",
    projectId(path),
    name,
  ].join("\\");
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
  const saveSettingsFile = () =>
    sessionStorage.setItem(DISK_KEY, JSON.stringify(Object.fromEntries(settings)));
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
    projects: new MockProjects(),
    saveDelayMs: 0,
    jobs: new MockJobs(),
    outputs: new Set(),
    presets: new MockPresets(),
    runAnswerDelayMs: 0,
    focused: true,
    taskbar: [],
    notifications: [],
    revealed: [],
    opened: [],
    backendSettings: new MockSettings(settings, saveSettingsFile),
  };
  installNotificationShim();
  let installedAt = performance.now();
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
  installSchemeFetch(backend.schemeRequests, {
    frames: (id) => frameCount(projects.get(id) ?? ""),
    leak: (id, index) => {
      const path = projects.get(id);
      const leak =
        path === undefined ? undefined : backend.projects.state(path).report?.leaks[index];
      return leak ? { frame: leak.frame_range[0], rect: leak.rect } : null;
    },
  });

  /** Writes the export of `path` and answers the render's summary. */
  const render = (path: string, output: ExportSettings) => {
    const { report, unverified } = backend.projects.state(path);
    const state = unverified.length > 0 ? "Unverified" : (report?.summary.state ?? "Unverified");
    const { media } = mockProject(path);
    const written = mockOutput(path, media, output, backend.outputs, state === "Clean").path;
    backend.outputs.add(pathKey(written));
    backend.projects.state(path).lastOutput = written;
    return {
      frames: frameCount(path),
      elapsed_ms: backend.jobs.durationMs,
      output: written,
      verify: state,
      warnings: 0,
    };
  };

  mockIPC(async (cmd, args) => {
    if (events.handles(cmd)) return events.invoke(cmd, args as Parameters<typeof events.invoke>[1]);
    backend.calls.push({ cmd, args });
    if (backend.backendSettings.handles(cmd)) {
      return backend.backendSettings.invoke(cmd, args, {
        openPaths: [...projects.values()],
        idle: backend.jobs.isIdle(),
        reprobe: () => {
          installedAt = performance.now();
        },
      });
    }
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
        return probeMedia(
          (args as { paths: string[] }).paths,
          backend.folders,
          (path) => backend.projects.state(path).analyzed,
        );
      case "plugin:dialog|open": {
        // A single-choice dialog answers with one path, like the real plugin.
        const { options } = args as { options?: { multiple?: boolean } };
        if (options?.multiple) return backend.dialogResult;
        return backend.dialogResult?.[0] ?? null;
      }
      case "get_capabilities":
        return performance.now() - installedAt < backend.capabilitiesDelayMs
          ? { ...backend.capabilities, probe_state: "Pending", encoders: [] }
          : backend.capabilities;
      case "get_session":
        return backend.sessionFile() ?? [];
      case "open_project": {
        const { path } = args as { path: string };
        projects.set(projectId(path), path);
        const project = mockProject(path);
        const { unverified, analyzed, report } = backend.projects.state(path);
        return {
          ...project,
          analyzed,
          verify: {
            plan_revision: report?.plan_revision ?? 0,
            state: report?.summary.state ?? "Unverified",
            unverified_ranges: unverified,
            report: report ? project.verify.report : null,
          },
        };
      }
      case "get_tracks":
        return backend.projects.state(pathOf(args)).set;
      case "set_overrides": {
        const { baseRev, overrides } = args as { baseRev: number; overrides: Override[] };
        const path = pathOf(args);
        await delay(backend.saveDelayMs);
        return backend.projects.setOverrides(path, baseRev, overrides);
      }
      case "get_pts":
        return mockPts(pathOf(args));
      case "get_timeline_density": {
        const path = pathOf(args);
        const { buckets } = args as { buckets: number };
        return mockDensity(path, buckets, backend.projects.state(path).set);
      }
      case "get_verify_report":
        return backend.projects.state(pathOf(args)).report;
      case "start_analysis": {
        const path = pathOf(args);
        const { regions, verify, onEvent } = args as unknown as AnalysisRequest & {
          onEvent: MockChannel;
        };
        const frames = frameCount(path);
        return backend.jobs.start("Analyze", frames, onEvent, () => {
          backend.projects.analyze(path, new Set(regions.enabled), verify.enabled);
          const { report } = backend.projects.state(path);
          return {
            frames,
            elapsed_ms: backend.jobs.durationMs,
            // Like the engine: an analysis names the project file it wrote.
            output: cacheFile(path, "project.json"),
            verify: verify.enabled ? (report?.summary.state ?? null) : null,
            warnings: 0,
          };
        });
      }
      case "get_output_preview": {
        const path = pathOf(args);
        const { settings: output } = args as { settings: ExportSettings };
        const { report, unverified } = backend.projects.state(path);
        const clean = report?.summary.state === "Clean" && unverified.length === 0;
        return mockOutput(path, mockProject(path).media, output, backend.outputs, clean);
      }
      case "start_render": {
        const path = pathOf(args);
        const { settings: output, onEvent } = args as unknown as RenderRequest & {
          onEvent: MockChannel;
        };
        return backend.jobs.start("Render", frameCount(path), onEvent, () => render(path, output));
      }
      case "start_reverify": {
        const path = pathOf(args);
        const { onEvent } = args as unknown as { onEvent: MockChannel };
        const frames = backend.projects
          .state(path)
          .unverified.reduce((sum, [f0, f1]) => sum + Math.max(f1 - f0 + 1, 1), 0);
        return backend.jobs.start("ReverifyRanges", frames, onEvent, () => {
          backend.projects.reverify(path);
          const { report } = backend.projects.state(path);
          return {
            frames,
            elapsed_ms: backend.jobs.durationMs,
            // Like the engine: a re-verification names the report it wrote.
            output: cacheFile(path, `verify-${report?.plan_revision ?? 0}.json`),
            verify: report?.summary.state ?? null,
            warnings: 0,
          };
        });
      }
      case "start_run": {
        const { files, onEvent } = args as unknown as { files: RunFile[]; onEvent: MockChannel };
        const entries = files.map((file) => {
          const path = pathOf(file);
          const { analyzed, unverified } = backend.projects.state(path);
          // Like the backend: a file whose analysis is current is only rendered.
          const current = analyzed && unverified.length === 0;
          return {
            kind: current ? ("Render" as const) : ("Run" as const),
            totalFrames: frameCount(path),
            finish: () => {
              if (!current) {
                backend.projects.analyze(path, new Set(file.regions.enabled), file.verify.enabled);
              }
              return render(path, file.settings);
            },
          };
        });
        const jobs = backend.jobs.enqueue(entries, onEvent);
        await delay(backend.runAnswerDelayMs);
        return jobs;
      }
      case "pause_queue":
        backend.jobs.paused = true;
        return null;
      case "resume_queue":
        backend.jobs.paused = false;
        return null;
      case "plugin:window|set_progress_bar":
        backend.taskbar.push((args as { value: TaskbarCall }).value);
        return null;
      case "plugin:window|is_focused":
        return backend.focused;
      case "plugin:notification|is_permission_granted":
        return true;
      case "plugin:notification|request_permission":
        return "granted";
      case "plugin:notification|notify": {
        const { options } = args as { options: { title: string; body?: string } };
        backend.notifications.push({ title: options.title, body: options.body ?? "" });
        return null;
      }
      case "plugin:opener|reveal_item_in_dir":
        backend.revealed.push(...(args as { paths: string[] }).paths);
        return null;
      case "plugin:opener|open_url":
        backend.opened.push((args as { url: string }).url);
        return null;
      case "accept_report": {
        const path = pathOf(args);
        const { reportId } = args as { reportId: string };
        const renamed = backend.projects.accept(path, reportId, mockProject(path).verify.report);
        backend.outputs.add(pathKey(renamed));
        return renamed;
      }
      case "list_presets":
        return backend.presets.list();
      case "save_preset":
        return backend.presets.save((args as { preset: Preset }).preset);
      case "delete_preset":
        return backend.presets.delete((args as { id: string }).id);
      case "import_preset":
        return backend.presets.import((args as { path: string }).path);
      case "export_preset": {
        const { id, path, includeKeptNames } = args as {
          id: string;
          path: string;
          includeKeptNames: boolean;
        };
        backend.presets.export(id, path, includeKeptNames);
        return null;
      }
      case "plugin:dialog|save":
        return backend.dialogResult?.[0] ?? null;
      case "cancel_job":
        backend.jobs.cancel((args as { job: string }).job);
        return null;
      case "close_project":
        return null;
      case "save_session":
        sessionStorage.setItem(SESSION_KEY, JSON.stringify((args as { entries: unknown }).entries));
        return null;
      case "plugin:window|minimize":
      case "plugin:window|start_dragging":
      case "plugin:log|log":
        return null;
      default:
        if (cmd.startsWith("plugin:store|")) return backend.backendSettings.store(cmd, args);
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
