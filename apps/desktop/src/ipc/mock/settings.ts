/**
 * The backend's settings sections in the mock (`get_settings` and the
 * `save_*` commands), read from and written to the mocked settings file
 * like the Rust side (`src-tauri/src/settings.rs`): a missing or invalid
 * section reads as its default, saves are checked first and refused for a
 * file of another schema major.
 */
import type { AppError } from "@/bindings/AppError";
import type { CacheInfo } from "@/bindings/CacheInfo";
import type { Defaults } from "@/bindings/Defaults";
import type { LogLevel } from "@/bindings/LogLevel";
import type { Privacy } from "@/bindings/Privacy";
import type { Processing } from "@/bindings/Processing";
import type { StoredSettings } from "@/bindings/StoredSettings";
import { DEFAULT_DEFAULTS, DEFAULT_PRIVACY, DEFAULT_PROCESSING } from "@/lib/app-defaults";
import { templateProblem } from "@/lib/export-options";
import { pathKey } from "@/lib/media";

/** Schema id the backend writes. */
const SCHEMA = "blurrd-settings/1";
const LOG_LEVELS: ReadonlySet<unknown> = new Set<LogLevel>([
  "Error",
  "Warn",
  "Info",
  "Debug",
  "Trace",
]);

/** The mocked cache folder (`%LOCALAPPDATA%\blurrd`). */
export const MOCK_CACHE_DIR = "C:\\Users\\me\\AppData\\Local\\blurrd";
/** The mocked log folder. */
export const MOCK_LOG_DIR = "C:\\Users\\me\\AppData\\Local\\io.github.konbuku.blurrd\\logs";

function object(value: unknown): Record<string, unknown> | null {
  return typeof value === "object" && value !== null && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : null;
}

function invalid(detail: string): AppError {
  return { kind: "InvalidProject", message: "The settings are invalid.", detail };
}

/** Why processing settings cannot be saved, or `null`. */
function processingProblem(processing: Processing): string | null {
  return processing.parallel_jobs === 1 || processing.parallel_jobs === 2
    ? null
    : `parallel jobs ${processing.parallel_jobs} must be 1 or 2`;
}

/** Why defaults cannot be saved, or `null`. */
function defaultsProblem(defaults: Defaults): string | null {
  const { export: output } = defaults;
  if (output.preset < 1 || output.preset > 7) return `preset ${output.preset} is outside 1..=7`;
  if (templateProblem(output.output.template) !== null) {
    return `the file name template \`${output.output.template}\` does not name a valid file`;
  }
  const folder = output.output.folder;
  if (typeof folder !== "string" && !/^[a-z]:\\|^\\\\/i.test(folder.Folder)) {
    return "Only absolute paths are accepted.";
  }
  return null;
}

function readProcessing(value: unknown): Processing {
  const raw = object(value);
  if (!raw) return DEFAULT_PROCESSING;
  const processing = { ...DEFAULT_PROCESSING, ...raw } as Processing;
  return processingProblem(processing) === null ? processing : DEFAULT_PROCESSING;
}

/**
 * The defaults section with missing fields (also of the export section and
 * its output naming) filled in from the defaults, like the Rust types.
 */
function readDefaults(value: unknown): Defaults {
  const raw = object(value);
  if (!raw) return DEFAULT_DEFAULTS;
  const rawExport = object(raw.export);
  const exportSettings = rawExport && {
    ...DEFAULT_DEFAULTS.export,
    ...rawExport,
    output: { ...DEFAULT_DEFAULTS.export.output, ...object(rawExport.output) },
  };
  const defaults = {
    ...DEFAULT_DEFAULTS,
    ...raw,
    ...(exportSettings ? { export: exportSettings } : {}),
  } as Defaults;
  return defaultsProblem(defaults) === null ? defaults : DEFAULT_DEFAULTS;
}

function readPrivacy(value: unknown): Privacy {
  const level = object(value)?.log_level;
  return LOG_LEVELS.has(level) ? { log_level: level as LogLevel } : DEFAULT_PRIVACY;
}

/** Name of the settings file, as opened by the real shell. */
const SETTINGS_FILE = "settings.json";
/** Resource id of the settings store. */
const SETTINGS_RID = 1;

/** The object members of a store command's arguments. */
interface StoreArgs {
  readonly rid?: number;
  readonly path?: string;
  readonly key?: string;
  readonly value?: unknown;
}

/** Commands of the backend's settings sections. */
const COMMANDS: ReadonlySet<string> = new Set([
  "get_settings",
  "save_processing",
  "save_defaults",
  "save_privacy",
  "get_cache_info",
  "clear_cache",
  "open_log_dir",
  "refresh_capabilities",
]);

/** What the settings commands need from the rest of the mock backend. */
export interface SettingsContext {
  /** Paths of the open projects. */
  readonly openPaths: readonly string[];
  /** Whether no job runs or waits. */
  readonly idle: boolean;
  /** Starts the capability probe again. */
  readonly reprobe: () => void;
}

/** The mocked backend sections over the settings file's entries. */
export class MockSettings {
  /** Processing the "running engine" was created with. */
  activeProcessing: Processing;
  /** Analysed recordings in the cache and their size. */
  cachedProjects = 3;
  cachedBytes = 182_452_224;
  /** How often the log folder was opened. */
  logDirOpened = 0;

  constructor(
    private readonly entries: Map<string, unknown>,
    private readonly save: () => void,
  ) {
    this.activeProcessing = readProcessing(entries.get("processing"));
  }

  private writable(): boolean {
    const schema = this.entries.get("schema");
    return schema === undefined || schema === SCHEMA;
  }

  /** `get_settings`. */
  stored(): StoredSettings {
    return {
      processing: readProcessing(this.entries.get("processing")),
      active_processing: this.activeProcessing,
      defaults: readDefaults(this.entries.get("defaults")),
      privacy: readPrivacy(this.entries.get("privacy")),
      writable: this.writable(),
    };
  }

  private write(key: string, value: unknown) {
    if (!this.writable()) {
      throw {
        kind: "Io",
        message: "The settings could not be saved.",
        detail: "settings.json was written by another version of blurrd",
      } satisfies AppError;
    }
    this.entries.set("schema", SCHEMA);
    this.entries.set("version", 1);
    this.entries.set(key, value);
    this.save();
  }

  /** `save_processing`. */
  saveProcessing(processing: Processing) {
    const problem = processingProblem(processing);
    if (problem !== null) throw invalid(problem);
    this.write("processing", processing);
  }

  /** `save_defaults`: answers the defaults as stored. */
  saveDefaults(defaults: Defaults): Defaults {
    const problem = defaultsProblem(defaults);
    if (problem !== null) throw invalid(problem);
    this.write("defaults", defaults);
    return defaults;
  }

  /** `save_privacy`. */
  savePrivacy(privacy: Privacy) {
    if (!LOG_LEVELS.has(privacy.log_level)) throw invalid("unknown log level");
    this.write("privacy", privacy);
  }

  /** `get_cache_info`. */
  cacheInfo(): CacheInfo {
    return {
      cache_dir: MOCK_CACHE_DIR,
      projects: this.cachedProjects,
      project_bytes: this.cachedBytes,
      log_dir: MOCK_LOG_DIR,
    };
  }

  /** `clear_cache`: keeps the analyses of the open projects. */
  clear(open: number): CacheInfo {
    this.cachedProjects = Math.min(this.cachedProjects, open);
    this.cachedBytes = this.cachedProjects * 4_194_304;
    return this.cacheInfo();
  }

  /** Answers the store plugin's commands on the settings file. */
  store(cmd: string, rawArgs: unknown): unknown {
    const args = rawArgs as StoreArgs;
    if (cmd === "plugin:store|get_store") return args.path === SETTINGS_FILE ? SETTINGS_RID : null;
    if (args.rid !== SETTINGS_RID) throw new Error(`mock backend: unknown store ${args.rid}`);
    switch (cmd) {
      case "plugin:store|get":
        return [this.entries.get(args.key ?? ""), this.entries.has(args.key ?? "")];
      case "plugin:store|set":
        this.entries.set(args.key ?? "", args.value);
        return null;
      case "plugin:store|save":
        this.save();
        return null;
      default:
        throw new Error(`mock backend: unknown command ${cmd}`);
    }
  }

  /** Whether `cmd` is one of the settings commands. */
  handles(cmd: string): boolean {
    return COMMANDS.has(cmd);
  }

  /** Answers a settings command like the real shell. */
  invoke(cmd: string, args: unknown, context: SettingsContext): unknown {
    switch (cmd) {
      case "get_settings":
        return this.stored();
      case "save_processing":
        this.saveProcessing((args as { processing: Processing }).processing);
        return null;
      case "save_defaults":
        return this.saveDefaults((args as { defaults: Defaults }).defaults);
      case "save_privacy":
        this.savePrivacy((args as { privacy: Privacy }).privacy);
        return null;
      case "get_cache_info":
        return this.cacheInfo();
      case "clear_cache": {
        const { keep } = args as { keep: string[] };
        return this.clear(new Set([...context.openPaths, ...keep].map(pathKey)).size);
      }
      case "open_log_dir":
        this.logDirOpened += 1;
        return null;
      case "refresh_capabilities":
        if (!context.idle) {
          throw {
            kind: "Internal",
            message: "Wait until the running jobs are done.",
            detail: null,
          } satisfies AppError;
        }
        context.reprobe();
        return null;
      default:
        throw new Error(`mock backend: unknown command ${cmd}`);
    }
  }
}
