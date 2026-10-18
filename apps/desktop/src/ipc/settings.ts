/**
 * The settings file (`settings.json`, schema `blurrd-settings/1`, plan
 * 6.3) through the store plugin. The Rust side opens the store at start;
 * the frontend may only look it up, read, write and save it. The file is
 * a JSON object whose top-level keys are the fields of the `Settings`
 * binding; the frontend writes the sections it owns (appearance,
 * general) and leaves every other key untouched. The backend's sections
 * go through commands, so Rust checks them (`getStoredSettings` and the
 * `save*` wrappers below).
 */
import { invoke } from "@tauri-apps/api/core";
import { getStore, type Store } from "@tauri-apps/plugin-store";

import type { Appearance } from "@/bindings/Appearance";
import type { CacheInfo } from "@/bindings/CacheInfo";
import type { Defaults } from "@/bindings/Defaults";
import type { General } from "@/bindings/General";
import type { Privacy } from "@/bindings/Privacy";
import type { Processing } from "@/bindings/Processing";
import type { Settings } from "@/bindings/Settings";
import type { StoredSettings } from "@/bindings/StoredSettings";
import { parseAppearance } from "@/lib/appearance";
import { parseGeneral } from "@/lib/general";

/** File name, as opened by the Rust side (`SETTINGS_FILE`). */
const FILE = "settings.json";

/** Schema id and revision this frontend writes. */
const SCHEMA = "blurrd-settings/1";
const REVISION = 1;

/** The open settings file. */
export interface SettingsFile {
  /** Appearance, defaults filled in. */
  readonly appearance: Appearance;
  /** General behaviour, defaults filled in. */
  readonly general: General;
  /**
   * Whether the file may be written. A file of another schema major
   * (written by a newer blurrd) is only read, never overwritten.
   */
  readonly writable: boolean;
  /** Replaces the appearance section and writes the file. */
  saveAppearance(appearance: Appearance): Promise<void>;
  /** Replaces the general section and writes the file. */
  saveGeneral(general: General): Promise<void>;
}

/** Writes one top-level section together with the schema header. */
async function saveSection<K extends keyof Settings>(
  store: Store,
  key: K,
  value: Settings[K],
): Promise<void> {
  await store.set("schema", SCHEMA);
  await store.set("version", REVISION);
  await store.set(key, value);
  await store.save();
}

/**
 * Opens the settings file. A missing file reads as defaults and is created
 * on the first save. When the Rust side could not open the file (it is
 * unreadable), the defaults apply and nothing is written.
 */
export async function openSettings(): Promise<SettingsFile> {
  const store = await getStore(FILE);
  if (!store) return readOnlyDefaults("the settings file could not be opened");
  const schema = await store.get<unknown>("schema");
  const writable = schema === undefined || schema === SCHEMA;
  const appearance = parseAppearance(await store.get<unknown>("appearance"));
  const general = parseGeneral(await store.get<unknown>("general"));
  const save = async <K extends keyof Settings>(key: K, value: Settings[K]) => {
    if (!writable) {
      throw new Error(`the settings file has schema ${String(schema)}, expected ${SCHEMA}`);
    }
    await saveSection(store, key, value);
  };

  return {
    appearance,
    general,
    writable,
    saveAppearance: (next) => save("appearance", next),
    saveGeneral: (next) => save("general", next),
  };
}

/** Default settings that refuse to save, explaining why. */
function readOnlyDefaults(reason: string): SettingsFile {
  return {
    appearance: parseAppearance(undefined),
    general: parseGeneral(undefined),
    writable: false,
    saveAppearance: () => Promise.reject(new Error(reason)),
    saveGeneral: () => Promise.reject(new Error(reason)),
  };
}

/**
 * The sections the backend owns (plan 6.3): processing, the defaults for
 * new files and privacy. The backend reads them leniently, checks every
 * value before it writes it and refuses to write a file of another
 * schema major.
 */
export function getStoredSettings(): Promise<StoredSettings> {
  return invoke<StoredSettings>("get_settings");
}

/** Saves the processing settings; the engine uses them from the next start. */
export function saveProcessing(processing: Processing): Promise<void> {
  return invoke<void>("save_processing", { processing });
}

/**
 * Saves the defaults for new files; resolves to them as stored (a chosen
 * output folder in its canonical form). Rejects with an `AppError` for an
 * invalid value or a folder the user did not choose.
 */
export function saveDefaults(defaults: Defaults): Promise<Defaults> {
  return invoke<Defaults>("save_defaults", { defaults });
}

/** Saves the privacy settings; the log level applies at once. */
export function savePrivacy(privacy: Privacy): Promise<void> {
  return invoke<void>("save_privacy", { privacy });
}

/** The cache and log folders and how much the cached analyses take. */
export function getCacheInfo(): Promise<CacheInfo> {
  return invoke<CacheInfo>("get_cache_info");
}

/**
 * Deletes the cached analyses except those of the open projects and of
 * `keep` (the queued files); resolves to what is left.
 */
export function clearCache(keep: readonly string[]): Promise<CacheInfo> {
  return invoke<CacheInfo>("clear_cache", { keep });
}

/** Opens the log folder in Explorer. */
export function openLogDir(): Promise<void> {
  return invoke<void>("open_log_dir");
}

/**
 * Forgets the cached decoder benchmark and encoder probe and probes this
 * PC again (`get_capabilities` is pending until it is done). Rejects while
 * a job runs or waits.
 */
export function refreshCapabilities(): Promise<void> {
  return invoke<void>("refresh_capabilities");
}
