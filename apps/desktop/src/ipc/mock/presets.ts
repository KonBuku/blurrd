/**
 * Presets of the mock backend (plan 6.5): the built-ins, user presets kept
 * in session storage (they survive reloads like `presets.json` survives
 * restarts), and preset files in the fake file system. Exported files
 * leave out the names kept readable unless asked to keep them.
 */
import type { AppError } from "@/bindings/AppError";
import type { PresetFile } from "@/bindings/PresetFile";
import { BUILTIN_PRESETS, withoutKeptNames, type PresetData } from "@/lib/presets";

/** Session storage key of the user presets. */
const STORAGE_KEY = "blurrd.mock.presets";

/** Schema id of preset files. */
const SCHEMA = "blurrd-presets/1";

function failure(kind: AppError["kind"], message: string): AppError {
  return { kind, message, detail: null };
}

/** A preset file's content (`blurrd-presets/1`) holding presets with screen surfaces. */
type MockPresetFile = Omit<PresetFile, "presets"> & { readonly presets: PresetData[] };

/** The presets of one mock backend. */
export class MockPresets {
  /** Files written by `export_preset` and read by `import_preset`: path → JSON text. */
  readonly files = new Map<string, string>();

  private user(): PresetData[] {
    const text = sessionStorage.getItem(STORAGE_KEY);
    return text === null ? [] : (JSON.parse(text) as PresetData[]);
  }

  private store(presets: readonly PresetData[]): PresetData[] {
    sessionStorage.setItem(STORAGE_KEY, JSON.stringify(presets));
    return this.list();
  }

  /** `list_presets`: built-ins first, then the user's presets. */
  list(): PresetData[] {
    return [...BUILTIN_PRESETS, ...this.user()];
  }

  /** `save_preset`: adds or replaces a user preset. */
  save(preset: PresetData): PresetData[] {
    if (BUILTIN_PRESETS.some((builtin) => builtin.id === preset.id)) {
      throw failure("InvalidProject", "Built-in presets cannot be changed.");
    }
    if (preset.name.trim() === "") throw failure("InvalidProject", "The preset needs a name.");
    const user = this.user();
    const index = user.findIndex((candidate) => candidate.id === preset.id);
    const saved = { ...preset, builtin: false };
    return this.store(index === -1 ? [...user, saved] : user.with(index, saved));
  }

  /** `delete_preset`: removes a user preset. */
  delete(id: string): PresetData[] {
    if (BUILTIN_PRESETS.some((builtin) => builtin.id === id)) {
      throw failure("InvalidProject", "Built-in presets cannot be deleted.");
    }
    return this.store(this.user().filter((preset) => preset.id !== id));
  }

  /** `export_preset`: writes one preset to `path`. */
  export(id: string, path: string, includeKeptNames: boolean): void {
    const preset = this.list().find((candidate) => candidate.id === id);
    if (!preset) throw failure("NotFound", "The preset does not exist.");
    const file: MockPresetFile = {
      schema: SCHEMA,
      version: 1,
      presets: [includeKeptNames ? preset : withoutKeptNames(preset)],
    };
    this.files.set(path, JSON.stringify(file));
  }

  /** `import_preset`: adds the presets of the file at `path` as user presets. */
  import(path: string): PresetData[] {
    const text = this.files.get(path);
    if (text === undefined) throw failure("NotFound", "The file does not exist.");
    const file = JSON.parse(text) as Partial<MockPresetFile>;
    if (file.schema !== SCHEMA || !Array.isArray(file.presets)) {
      throw failure("InvalidProject", "This is not a blurrd preset file.");
    }
    const taken = new Set(this.list().map((preset) => preset.id));
    const added = file.presets.map((preset) => {
      let id = preset.id;
      for (let n = 2; taken.has(id); n += 1) id = `${preset.id}-${n}`;
      taken.add(id);
      return { ...preset, id, builtin: false };
    });
    this.store([...this.user(), ...added]);
    return added;
  }
}
