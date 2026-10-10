import type { ExportSettings } from "@/bindings/ExportSettings";

/** Changes the export settings of the inspector's targets. */
export type ExportChange = (patch: (current: ExportSettings) => ExportSettings) => void;

/** Props of an export inspector section. */
export interface ExportSectionProps {
  /** The shown export settings. */
  readonly output: ExportSettings;
  readonly change: ExportChange;
}

/** A setter of one export field through `change`. */
export function fieldSetter(change: ExportChange) {
  return <K extends keyof ExportSettings>(key: K, value: ExportSettings[K]) =>
    change((current) => ({ ...current, [key]: value }));
}
