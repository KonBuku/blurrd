// SPDX-License-Identifier: GPL-3.0-only
// Copyright 2026 The blurrd Authors
//
// Rasterise SVG files to PNG with resvg (no system fonts: every brand SVG
// contains outlined text only).
//
// Usage: node render.mjs <deps-dir> <width> <input.svg> <output.png> [<input.svg> <output.png> ...]
//
// <deps-dir> is the folder that contains `node_modules/@resvg/resvg-js`
// (see README.md, "Regenerating the assets").

import { readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { join, resolve } from "node:path";

const [, , depsDir, widthArg, ...pairs] = process.argv;
const width = Number(widthArg);

if (!depsDir || !Number.isInteger(width) || width <= 0 || pairs.length === 0 || pairs.length % 2 !== 0) {
  console.error("usage: node render.mjs <deps-dir> <width> <input.svg> <output.png> [...]");
  process.exit(2);
}

const require = createRequire(join(resolve(depsDir), "package.json"));
const { Resvg } = require("@resvg/resvg-js");

for (let i = 0; i < pairs.length; i += 2) {
  const [input, output] = [pairs[i], pairs[i + 1]];
  const resvg = new Resvg(readFileSync(input), {
    fitTo: { mode: "width", value: width },
    font: { loadSystemFonts: false },
  });
  writeFileSync(output, resvg.render().asPng());
}
