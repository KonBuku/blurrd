// Applies the theme cached by src/lib/appearance.ts (`blurrd.appearance`
// in local storage) before anything paints; without a cache the default
// dark theme of index.html stays. The app applies the settings file once
// it loads. A file, not an inline script, so the CSP stays as configured.
try {
  var theme = JSON.parse(localStorage.getItem("blurrd.appearance") || "{}").theme;
  var light =
    theme === "Light" ||
    (theme === "System" && !matchMedia("(prefers-color-scheme: dark)").matches);
  if (light) document.documentElement.classList.remove("dark");
} catch {
  // No cache or no storage: the dark default stays.
}
