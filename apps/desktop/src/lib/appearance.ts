import type { Appearance } from "@/bindings/Appearance";
import type { ReduceMotion } from "@/bindings/ReduceMotion";
import type { Theme } from "@/bindings/Theme";

/** Appearance of a fresh install (matches the Rust `Appearance::default`). */
export const DEFAULT_APPEARANCE: Appearance = {
  theme: "Dark",
  ui_font_size: 14,
  reduce_motion: "System",
  timeline_thumbs: true,
};

/** Interface text sizes the user can pick, in pixels. */
export const UI_FONT_SIZES: readonly number[] = [12, 13, 14, 15, 16];

const FONT_SIZES: ReadonlySet<unknown> = new Set(UI_FONT_SIZES);
const THEMES: ReadonlySet<unknown> = new Set<Theme>(["System", "Light", "Dark"]);
const MOTIONS: ReadonlySet<unknown> = new Set<ReduceMotion>(["System", "On", "Off"]);

/** Query that matches when the operating system uses a dark theme. */
const DARK_QUERY = "(prefers-color-scheme: dark)";

/** Local storage key of the appearance copy used for the first paint. */
const CACHE_KEY = "blurrd.appearance";

function isTheme(value: unknown): value is Theme {
  return THEMES.has(value);
}

function isMotion(value: unknown): value is ReduceMotion {
  return MOTIONS.has(value);
}

/**
 * Reads an appearance from untrusted JSON (the settings file can be edited
 * by hand): every field that is missing or invalid takes its default, like
 * the Rust deserializer does.
 */
export function parseAppearance(value: unknown): Appearance {
  const raw = typeof value === "object" && value !== null ? (value as Record<string, unknown>) : {};
  return {
    theme: isTheme(raw.theme) ? raw.theme : DEFAULT_APPEARANCE.theme,
    ui_font_size: FONT_SIZES.has(raw.ui_font_size)
      ? (raw.ui_font_size as number)
      : DEFAULT_APPEARANCE.ui_font_size,
    reduce_motion: isMotion(raw.reduce_motion)
      ? raw.reduce_motion
      : DEFAULT_APPEARANCE.reduce_motion,
    timeline_thumbs:
      typeof raw.timeline_thumbs === "boolean"
        ? raw.timeline_thumbs
        : DEFAULT_APPEARANCE.timeline_thumbs,
  };
}

/**
 * Applies an appearance to the document and keeps following the
 * operating system where it says "System":
 *
 * - theme: the `.dark` class on `root` switches the tokens in
 *   `styles/globals.css`;
 * - interface size: `--ui-font-size`, which drives every `text-ui-*` size;
 * - reduce motion: `data-motion="reduced"` or `"full"` overrides the
 *   system preference, no attribute follows it.
 *
 * Returns a function that stops following the system theme.
 */
export function applyAppearance(
  appearance: Appearance,
  root: HTMLElement = document.documentElement,
  media: MediaQueryList = window.matchMedia(DARK_QUERY),
): () => void {
  root.style.setProperty("--ui-font-size", `${appearance.ui_font_size}px`);
  if (appearance.reduce_motion === "System") delete root.dataset.motion;
  else root.dataset.motion = appearance.reduce_motion === "On" ? "reduced" : "full";

  const applyTheme = () => {
    const dark = appearance.theme === "Dark" || (appearance.theme === "System" && media.matches);
    root.classList.toggle("dark", dark);
  };
  applyTheme();
  if (appearance.theme !== "System") return () => undefined;
  media.addEventListener("change", applyTheme);
  return () => media.removeEventListener("change", applyTheme);
}

/**
 * The appearance cached for the first paint, read synchronously before the
 * settings file has loaded so the window does not flash the wrong theme.
 */
export function cachedAppearance(): Appearance {
  try {
    const text = localStorage.getItem(CACHE_KEY);
    return text === null ? DEFAULT_APPEARANCE : parseAppearance(JSON.parse(text));
  } catch {
    // Missing, blocked or corrupt cache: the settings file still loads.
    return DEFAULT_APPEARANCE;
  }
}

/** Caches the appearance for the next first paint. */
export function cacheAppearance(appearance: Appearance): void {
  try {
    localStorage.setItem(CACHE_KEY, JSON.stringify(appearance));
  } catch {
    // Storage unavailable: the next start paints the default theme first.
  }
}
