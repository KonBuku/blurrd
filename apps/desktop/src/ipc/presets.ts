/**
 * The preset commands (plan 9.1): the presets live in `presets.json` in
 * the app config directory (plan 6.5); the built-ins are always listed
 * first and cannot be changed or deleted. Exported files
 * (`.blurrd-preset.json`, schema `blurrd-presets/1`) leave out the names
 * kept readable unless `includeKeptNames` is set.
 */
import { invoke } from "@tauri-apps/api/core";

import type { PresetData } from "@/lib/presets";

/** Every preset, built-ins first. */
export function listPresets(): Promise<PresetData[]> {
  return invoke<PresetData[]>("list_presets");
}

/** Adds a user preset or replaces the one with the same id; answers the new list. */
export function savePreset(preset: PresetData): Promise<PresetData[]> {
  return invoke<PresetData[]>("save_preset", { preset });
}

/** Deletes a user preset; answers the new list. */
export function deletePreset(id: string): Promise<PresetData[]> {
  return invoke<PresetData[]>("delete_preset", { id });
}

/**
 * Adds the presets of a preset file chosen in the open dialog and answers
 * the added ones (ids that are taken get a new one).
 */
export function importPreset(path: string): Promise<PresetData[]> {
  return invoke<PresetData[]>("import_preset", { path });
}

/** Writes one preset to a file chosen in the save dialog. */
export function exportPreset(id: string, path: string, includeKeptNames: boolean): Promise<void> {
  return invoke<void>("export_preset", { id, path, includeKeptNames });
}
