// Applies the theme before anything paints. The Rust side hands the
// window's theme (from settings.json) to the page as
// `window.blurrdBootTheme` before any script runs, so the first paint and
// the window background always agree; outside the app (dev server, mock)
// the copy cached by src/lib/appearance.ts (`blurrd.appearance` in local
// storage) is used. Without either the default dark theme of index.html
// stays. A file, not an inline script, so the CSP stays as configured.
try {
  var theme =
    window.blurrdBootTheme || JSON.parse(localStorage.getItem("blurrd.appearance") || "{}").theme;
  var light =
    theme === "Light" ||
    (theme === "System" && !matchMedia("(prefers-color-scheme: dark)").matches);
  if (light) document.documentElement.classList.remove("dark");
} catch {
  // No cache or no storage: the dark default stays.
}
