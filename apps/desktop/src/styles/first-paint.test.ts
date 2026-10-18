// The colours a start paints before the stylesheet loads must be the
// window shell colours of globals.css: the Tauri window's background
// (tauri.conf.json, src-tauri/src/lib.rs) and public/boot.css. A stale
// copy brings back a flash of another colour. index.html has no inline
// <style> or <script>: Tauri would add a nonce to the CSP, which turns
// off the 'unsafe-inline' that React's style attributes need.
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

const root = join(import.meta.dirname, "..", "..");
const read = (...parts: string[]) => readFileSync(join(root, ...parts), "utf8");

const css = read("src", "styles", "globals.css");
const html = read("index.html");
const bootCss = read("public", "boot.css");
const bootJs = read("public", "boot.js");
const conf = JSON.parse(read("src-tauri", "tauri.conf.json")) as {
  app: { windows: { label: string; backgroundColor?: string; create?: boolean }[] };
};
const rust = read("src-tauri", "src", "lib.rs");

/** The window shell colour of the block that starts at `marker`. */
function shell(marker: string): string {
  const start = css.indexOf(marker);
  const end = css.indexOf("\n}", start);
  const match = /--color-background-win-alt:\s*(#[0-9a-f]{6});/i.exec(css.slice(start, end));
  if (!match?.[1]) throw new Error(`no window shell colour in ${marker}`);
  return match[1].toLowerCase();
}

/** `#rrggbb` as the Rust `Color(0xrr, 0xgg, 0xbb, 0xff)` literal. */
function rustColor(hex: string): string {
  const [r, g, b] = [1, 3, 5].map((i) => `0x${hex.slice(i, i + 2)}`);
  return `Color(${r}, ${g}, ${b}, 0xff)`;
}

describe("first paint", () => {
  const dark = shell(".dark {");
  const light = shell("@theme {");

  it("starts the window in the dark shell colour", () => {
    const main = conf.app.windows.find((window) => window.label === "main");
    expect(main?.backgroundColor?.toLowerCase()).toBe(dark);
    expect(rust).toContain(`const DARK_SHELL: Color = ${rustColor(dark)};`);
    expect(rust).toContain(`const LIGHT_SHELL: Color = ${rustColor(light)};`);
  });

  it("paints the shell colour of the cached theme before the app loads", () => {
    expect(html).toContain('<html lang="en" class="dark">');
    expect(html).toContain('<link rel="stylesheet" href="/boot.css" />');
    expect(html).toContain('<script src="/boot.js"></script>');
    expect(html).not.toMatch(/<style|<script>/);
    expect(bootCss).toMatch(new RegExp(`html \\{[^}]*background-color: ${dark};`));
    expect(bootCss).toMatch(
      new RegExp(`html:not\\(\\.dark\\) \\{[^}]*background-color: ${light};`),
    );
    expect(bootJs).toContain('localStorage.getItem("blurrd.appearance")');
  });
});
