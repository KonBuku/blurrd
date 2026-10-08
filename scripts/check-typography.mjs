#!/usr/bin/env node
// Checks that the desktop frontend uses only the `text-ui-*` type scale.
//
// Usage: pnpm lint:typography
//
// Every text size in apps/desktop/src comes from one interface-size variable
// (`--ui-font-size`, see apps/desktop/src/styles/globals.css), so the user's
// size setting scales the whole UI. Tailwind's default sizes (`text-sm`,
// `text-lg`, ...) and fixed arbitrary sizes (`text-[13px]`) bypass it. The
// stylesheet switches the default scale off, but a stray class would then
// silently render at the inherited size; this check makes it an error.

import { readdirSync, readFileSync } from "node:fs";
import { extname, join, relative } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = fileURLToPath(new URL("..", import.meta.url));
const SOURCE = join(ROOT, "apps", "desktop", "src");
const EXTENSIONS = new Set([".ts", ".tsx", ".css", ".html"]);

// A Tailwind size class not preceded by a word character or hyphen, so
// `text-ui-sm` and `context-sm` do not match.
const BANNED = [
  {
    pattern: /(?<![\w-])text-(?:xs|sm|base|lg|[2-9]?xl)\b/g,
    hint: "use a text-ui-* size",
  },
  {
    pattern: /(?<![\w-])text-\[\d+(?:\.\d+)?(?:px|rem|em)\]/g,
    hint: "use a text-ui-* size instead of a fixed size",
  },
];

/** Yields every checked file below `dir`. */
function* files(dir) {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) yield* files(path);
    else if (EXTENSIONS.has(extname(entry.name))) yield path;
  }
}

const problems = [];
for (const file of files(SOURCE)) {
  const lines = readFileSync(file, "utf8").split("\n");
  lines.forEach((line, index) => {
    for (const { pattern, hint } of BANNED) {
      for (const match of line.matchAll(pattern)) {
        const where = `${relative(ROOT, file).replaceAll("\\", "/")}:${index + 1}:${match.index + 1}`;
        problems.push(`${where}: "${match[0]}" bypasses the type scale; ${hint}`);
      }
    }
  });
}

if (problems.length > 0) {
  for (const problem of problems) console.error(problem);
  console.error(`\n${problems.length} typography violation(s).`);
  process.exit(1);
}
process.stdout.write("typography: only text-ui-* sizes are used.\n");
