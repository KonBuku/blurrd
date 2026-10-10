import { create } from "zustand";

/** The About dialogs that open without a trigger (command palette). */
export type AboutDialog = "about" | "licenses";

interface AboutState {
  /** The dialog shown, if any. */
  readonly open: AboutDialog | null;
  readonly show: (dialog: AboutDialog) => void;
  readonly close: () => void;
}

/** Which About dialog the command palette opened. */
export const useAboutStore = create<AboutState>()((set) => ({
  open: null,
  show: (open) => set({ open }),
  close: () => set({ open: null }),
}));
