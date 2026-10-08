/** Query that matches when the operating system uses a dark theme. */
const DARK_QUERY = "(prefers-color-scheme: dark)";

/**
 * Keeps the `.dark` class on `root` in sync with the operating system theme
 * and returns a function that stops following it. The tokens in
 * `styles/globals.css` switch on that class.
 */
export function followSystemTheme(
  root: HTMLElement = document.documentElement,
  media: MediaQueryList = window.matchMedia(DARK_QUERY),
): () => void {
  const apply = () => {
    root.classList.toggle("dark", media.matches);
  };
  apply();
  media.addEventListener("change", apply);
  return () => media.removeEventListener("change", apply);
}
