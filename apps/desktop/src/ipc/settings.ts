/**
 * The settings file (`settings.json`, schema `blurrd-settings/1`, plan
 * 6.3) through the store plugin. The Rust side opens the store at start;
 * the frontend may only look it up, read, write and save it. The file is
 * a JSON object whose top-level keys are the fields of the `Settings`
 * binding; the frontend writes the sections it owns and leaves every
 * other key untouched.
 */
import { getStore, type Store } from "@tauri-apps/plugin-store";

import type { Appearance } from "@/bindings/Appearance";
import type { General } from "@/bindings/General";
import type { Settings } from "@/bindings/Settings";
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
