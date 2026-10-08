// Contrast checks of the tokens in globals.css, for both themes: primary
// and secondary text must reach WCAG AA (4.5:1) on every surface it sits
// on; region chip text must reach 4.5:1 on its translucent surface over
// the colours chips sit on, and every region colour the non-text minimum
// (3:1) against them (overlay strokes, timeline lanes).
import { readFileSync } from "node:fs";
import { join } from "node:path";

import { describe, expect, it } from "vitest";

// Read from disk: Vitest does not process CSS, so a `?raw` import is empty.
const css = readFileSync(join(import.meta.dirname, "globals.css"), "utf8");

type Rgba = readonly [number, number, number, number];

/** Returns the `--color-*` declarations of the first block after `marker`. */
function block(marker: string): Map<string, string> {
  const start = css.indexOf(marker);
  if (start === -1) throw new Error(`globals.css has no ${marker} block`);
  const end = css.indexOf("\n}", start);
  const tokens = new Map<string, string>();
  for (const match of css.slice(start, end).matchAll(/--color-([\w-]+):\s*([^;]+);/g)) {
    tokens.set(match[1] ?? "", (match[2] ?? "").trim());
  }
  return tokens;
}

/** Parses `#rrggbb` or `rgba(r, g, b, a)`. */
function parse(value: string): Rgba {
  const hex = /^#([0-9a-f]{2})([0-9a-f]{2})([0-9a-f]{2})$/i.exec(value);
  if (hex)
    return [parseInt(hex[1] ?? "", 16), parseInt(hex[2] ?? "", 16), parseInt(hex[3] ?? "", 16), 1];
  const rgba = /^rgba\((\d+),\s*(\d+),\s*(\d+),\s*([\d.]+)\)$/.exec(value);
  if (rgba) return [Number(rgba[1]), Number(rgba[2]), Number(rgba[3]), Number(rgba[4])];
  throw new Error(`unsupported colour ${value}`);
}

/** Composites a translucent colour over an opaque one. */
function over([r, g, b, a]: Rgba, [br, bg, bb]: Rgba): Rgba {
  return [r * a + br * (1 - a), g * a + bg * (1 - a), b * a + bb * (1 - a), 1];
}

/** Linearizes one sRGB channel (0-255). */
function channel(value: number): number {
  const c = value / 255;
  return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
}

/** WCAG 2 relative luminance. */
function luminance([r, g, b]: Rgba): number {
  return 0.2126 * channel(r) + 0.7152 * channel(g) + 0.0722 * channel(b);
}

/** WCAG 2 contrast ratio of two opaque colours. */
function contrast(a: Rgba, b: Rgba): number {
  const [high, low] = [luminance(a), luminance(b)].toSorted((x, y) => y - x) as [number, number];
  return (high + 0.05) / (low + 0.05);
}

const FAMILIES = ["killfeed", "banner", "voice", "chat", "manual", "screens"] as const;
// Chips and lanes sit on cards, panels and the content background.
const GROUNDS = ["card", "panel", "background"] as const;

const light = block("@theme {");
const dark = new Map([...light, ...block("\n.dark {")]);

describe.each([
  ["light", light],
  ["dark", dark],
] as const)("region colours (%s theme)", (_theme, tokens) => {
  const color = (name: string) => {
    const value = tokens.get(name);
    if (value === undefined) throw new Error(`missing token --color-${name}`);
    return parse(value);
  };

  describe.each(FAMILIES)("%s", (family) => {
    it.each(GROUNDS)("chip text reaches 4.5:1 on its surface over %s", (ground) => {
      const surface = over(color(`region-${family}-surface`), color(ground));
      expect(contrast(color(`region-${family}`), surface)).toBeGreaterThanOrEqual(4.5);
    });

    it.each(GROUNDS)("the region colour reaches 3:1 against %s", (ground) => {
      expect(contrast(color(`region-${family}`), color(ground))).toBeGreaterThanOrEqual(3);
    });
  });

  // Text sits on the window shell (sidebar, title bar), the content
  // frames, cards and popovers, and on hovered or selected rows of each.
  const textGrounds = (): [string, Rgba][] => {
    const opaque = ["background-win-alt", "background", "panel", "card", "popover"];
    const overlays = ["surface", "hover", "selected"];
    return opaque.flatMap((ground): [string, Rgba][] => [
      [ground, color(ground)],
      ...overlays.map((overlay): [string, Rgba] => [
        `${overlay} on ${ground}`,
        over(color(overlay), color(ground)),
      ]),
    ]);
  };

  it.each(["foreground", "foreground-subtle"])("%s text reaches 4.5:1 on every surface", (text) => {
    for (const [name, ground] of textGrounds()) {
      const ratio = contrast(over(color(text), ground), ground);
      expect(ratio, `${text} on ${name}`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("keeps the focus ring at 3:1 against the content background", () => {
    expect(contrast(color("ring"), color("background"))).toBeGreaterThanOrEqual(3);
  });
});
