#!/usr/bin/env node
/**
 * Generates the third-party notices of everything blurrd ships:
 * `THIRD-PARTY-NOTICES.md` in the repository root and the data of the
 * app's open-source licences view
 * (`apps/desktop/src/features/about/licenses.json`).
 *
 * Sources, merged in this order:
 * 1. Programs and data the installer bundles: the FFmpeg build with its
 *    libraries (licence files of `third_party/ffmpeg`, versions from its
 *    BUILDINFO.txt), ONNX Runtime and DirectML (`third_party/ort.toml`,
 *    licence files of `third_party/ort`), the Visual C++ runtime DLLs next
 *    to them (`scripts/license-texts/msvc-runtime.txt`) and the OCR models
 *    (`models.toml`).
 * 2. Rust crates: `cargo about generate --format json` (cargo-about 0.9.2,
 *    `about.toml`) for the desktop app and the CLI.
 * 3. npm packages: the packages Vite bundles into the frontend, from
 *    Vite's own licence output (`build.license`), so packages that are
 *    installed but tree-shaken away are not listed.
 *
 * Usage:
 *   node scripts/licenses.mjs           write both files
 *   node scripts/licenses.mjs --check   fail when either file is stale
 *
 * Needs cargo-about on PATH, the FFmpeg build (`cargo xtask fetch-ffmpeg`)
 * and, once `third_party/ort.toml` exists, the ONNX Runtime packages
 * (`cargo xtask fetch-ort`). See docs/development.md ("Third-party
 * notices").
 */
import { spawnSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import { dirname, join, relative } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

import { parse as parseToml } from "smol-toml";

import { wrongText } from "./license-texts/check.mjs";
import { renderNotices } from "./license-texts/render.mjs";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const DESKTOP = join(ROOT, "apps", "desktop");
const NOTICES_FILE = join(ROOT, "THIRD-PARTY-NOTICES.md");
const DATA_FILE = join(DESKTOP, "src", "features", "about", "licenses.json");
/** The canonical Apache License 2.0 text, for the OCR models. */
const APACHE_TEXT = join(ROOT, "scripts", "license-texts", "Apache-2.0.txt");
/** The notice of the Visual C++ runtime DLLs `cargo xtask stage-bundle` ships. */
const VC_RUNTIME_TEXT = join(ROOT, "scripts", "license-texts", "msvc-runtime.txt");

/** Format tag of the data file, checked by the app. */
const SCHEMA = "blurrd-licenses/1";

/** The shipped Rust binaries whose dependency trees are listed. */
const RUST_ROOTS = ["apps/desktop/src-tauri/Cargo.toml", "crates/blurrd-cli/Cargo.toml"];

const REPOSITORY = "https://github.com/KonBuku/blurrd";

/**
 * Licences of the vcpkg ports in the FFmpeg build (third_party/ffmpeg/
 * licenses/<port>). A port without an entry stops the script, so a new
 * library in the build gets a reviewed licence before it ships.
 */
const FFMPEG_PORTS = {
  ffmpeg: { name: "FFmpeg", license: "LGPL-2.1-or-later", url: "https://ffmpeg.org" },
  "amd-amf": {
    name: "AMD Advanced Media Framework headers",
    license: "MIT",
    url: "https://github.com/GPUOpen-LibrariesAndSDKs/AMF",
  },
  dav1d: {
    name: "dav1d",
    license: "BSD-2-Clause",
    url: "https://code.videolan.org/videolan/dav1d",
  },
  ffnvcodec: {
    name: "NVIDIA codec headers (nv-codec-headers)",
    license: "MIT",
    url: "https://github.com/FFmpeg/nv-codec-headers",
  },
  libvpl: {
    name: "Intel Video Processing Library dispatcher",
    license: "MIT",
    url: "https://github.com/intel/libvpl",
  },
  zlib: { name: "zlib", license: "Zlib", url: "https://zlib.net" },
};

/** The standard MIT text, for npm packages that ship no licence file. */
const MIT_TEXT = `MIT License

Copyright (c) {holder}

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.`;

/** Fails the script with a message for the person running it. */
function fail(message) {
  console.error(`licenses: ${message}`);
  process.exit(1);
}

/** Reads a text file with Unix line ends and no surrounding blank lines. */
function readText(path) {
  return normalize(readFileSync(path, "utf8"));
}

/** Strips a byte order mark, converts line ends to LF and trims. */
function normalize(text) {
  const body = text.codePointAt(0) === 0xfeff ? text.slice(1) : text;
  return body.replace(/\r\n?/g, "\n").trim();
}

/** Collects components and deduplicates their licence texts. */
class Collector {
  /** @type {{ kind: string, name: string, version: string, license: string, url: string, source?: string, note?: string, texts: number[] }[]} */
  packages = [];
  /** @type {{ license: string, text: string }[]} */
  texts = [];
  #index = new Map();
  #seen = new Set();

  /** The index of `text`, adding it under `license` the first time it appears. */
  text(license, text) {
    const normalized = normalize(text);
    if (normalized === "") fail(`empty licence text for ${license}`);
    const wrong = wrongText(license, normalized);
    if (wrong !== null) fail(wrong);
    let index = this.#index.get(normalized);
    if (index === undefined) {
      index = this.texts.length;
      this.texts.push({ license, text: normalized });
      this.#index.set(normalized, index);
    }
    return index;
  }

  /** Adds a component once (by kind, name and version). */
  add(entry) {
    const key = `${entry.kind}:${entry.name}@${entry.version}`;
    if (this.#seen.has(key)) return;
    this.#seen.add(key);
    this.packages.push({ ...entry, texts: [...new Set(entry.texts)].toSorted((a, b) => a - b) });
  }
}

/** Versions of the installed vcpkg ports, from the build's BUILDINFO.txt. */
function vcpkgVersions(buildInfo) {
  const versions = new Map();
  for (const line of buildInfo.split("\n")) {
    const match = /^([a-z0-9-]+):x64-windows-blurrd\s+(\S+)/.exec(line);
    if (match) versions.set(match[1], match[2].replace(/#\d+$/, ""));
  }
  return versions;
}

/** FFmpeg and the libraries linked into its DLLs. */
function addFfmpeg(collector) {
  const pin = parseToml(readFileSync(join(ROOT, "third_party", "ffmpeg.toml"), "utf8"));
  const dir = join(ROOT, "third_party", "ffmpeg");
  const licenses = join(dir, "licenses");
  if (!existsSync(licenses)) fail("third_party/ffmpeg is missing; run `cargo xtask fetch-ffmpeg`");
  const versions = vcpkgVersions(readText(join(dir, "BUILDINFO.txt")));
  const release = `https://github.com/${pin.repository}/releases/download/${pin.tag}`;
  for (const port of readdirSync(licenses).toSorted()) {
    const info = FFMPEG_PORTS[port];
    if (!info) fail(`no reviewed licence for the FFmpeg library "${port}" (FFMPEG_PORTS)`);
    const version = port === "ffmpeg" ? pin.version : versions.get(port);
    if (!version) fail(`no version of "${port}" in third_party/ffmpeg/BUILDINFO.txt`);
    const text = collector.text(info.license, readText(join(licenses, port, "copyright")));
    collector.add({
      kind: "binary",
      name: info.name,
      version,
      license: info.license,
      url: info.url,
      ...(port === "ffmpeg"
        ? {
            source: `${release}/${pin.source_asset}`,
            note: `blurrd's own build ${pin.tag} without GPL or non-free components, linked dynamically: the FFmpeg DLLs next to blurrd.exe can be replaced with a compatible build. The source archive holds the FFmpeg source code, the build recipe and the patches.`,
          }
        : { note: "Part of blurrd's FFmpeg build." }),
      texts: [text],
    });
  }
}

/** ONNX Runtime and DirectML, once the OCR engine pins them. */
function addOnnxRuntime(collector) {
  const pinFile = join(ROOT, "third_party", "ort.toml");
  if (!existsSync(pinFile)) return;
  const pin = parseToml(readFileSync(pinFile, "utf8"));
  for (const item of pin.package ?? []) {
    const texts = (item.files ?? [])
      .filter((file) => file.to.startsWith("licenses/"))
      .map((file) => {
        const path = join(ROOT, "third_party", "ort", file.to);
        if (!existsSync(path)) fail(`${file.to} is missing; run \`cargo xtask fetch-ort\``);
        const label = /ThirdPartyNotices/i.test(file.to)
          ? `${item.id} third-party notices`
          : item.license;
        return collector.text(label, readText(path));
      });
    if (texts.length === 0) fail(`no licence file listed for ${item.id} in third_party/ort.toml`);
    collector.add({
      kind: "binary",
      name: item.id,
      version: item.version,
      license: item.license,
      url: `https://www.nuget.org/packages/${item.id}/${item.version}`,
      texts,
    });
  }
}

/**
 * The Visual C++ runtime DLLs the installer and the CLI archive ship next
 * to blurrd.exe for FFmpeg and ONNX Runtime (`cargo xtask stage-bundle`
 * copies them from the Visual Studio installation and checks that the
 * notice names each of them).
 */
function addVcRuntime(collector) {
  const text = collector.text("Microsoft Visual C++ Runtime", readText(VC_RUNTIME_TEXT));
  collector.add({
    kind: "binary",
    name: "Microsoft Visual C++ Runtime",
    version: "14 (Visual Studio 2022)",
    license: "Microsoft Visual Studio Distributable Code",
    url: "https://learn.microsoft.com/cpp/windows/redistributing-visual-cpp-files",
    texts: [text],
  });
}

/** The bundled OCR models and dictionary, once the OCR engine pins them. */
function addModels(collector) {
  const pinFile = join(ROOT, "models.toml");
  if (!existsSync(pinFile)) return;
  const pin = parseToml(readFileSync(pinFile, "utf8"));
  const { conversion } = pin;
  if (conversion.license !== "Apache-2.0") {
    fail(`models.toml: no licence text for "${conversion.license}"`);
  }
  // The canonical Apache License 2.0 (apache.org/licenses/LICENSE-2.0.txt),
  // pinned in the repository: blurrd's own LICENSE is a different licence.
  const text = collector.text("Apache-2.0", readText(APACHE_TEXT));
  for (const model of (pin.model ?? []).filter((item) => item.bundled)) {
    collector.add({
      kind: "model",
      name: model.file,
      version: conversion.revision,
      license: conversion.license,
      url: conversion.repository,
      note: `${conversion.upstream}; converted to ONNX with ${conversion.tool} by ${conversion.by} (${conversion.repository}, revision ${conversion.revision}).`,
      texts: [text],
    });
  }
}

/**
 * Runs cargo-about for one shipped binary. The JSON goes to a file:
 * cargo-about refuses to write redirected output when PowerShell started
 * it (as in CI).
 */
function cargoAbout(manifest) {
  const dir = mkdtempSync(join(tmpdir(), "blurrd-about-"));
  const out = join(dir, "about.json");
  try {
    const result = spawnSync(
      "cargo",
      ["about", "generate", "--format", "json", "--locked", "--fail", "-m", manifest, "-o", out],
      { cwd: ROOT, encoding: "utf8", maxBuffer: 256 * 1024 * 1024 },
    );
    if (result.error) fail(`cargo about could not start: ${result.error.message}`);
    if (result.status !== 0) {
      fail(`cargo about failed for ${manifest}:\n${result.stderr}\nInstall cargo-about 0.9.2.`);
    }
    return JSON.parse(readFileSync(out, "utf8"));
  } finally {
    rmSync(dir, { recursive: true, force: true });
  }
}

/** The crates of the desktop app and the CLI. */
function addCrates(collector) {
  const crates = new Map();
  for (const manifest of RUST_ROOTS) {
    const about = cargoAbout(manifest);
    for (const license of about.licenses) {
      const text = collector.text(license.id, license.text);
      for (const { crate } of license.used_by) {
        const key = `${crate.name}@${crate.version}`;
        const entry = crates.get(key) ?? {
          kind: "crate",
          name: crate.name,
          version: crate.version,
          license: crate.license ?? "",
          url: crate.repository ?? crate.homepage ?? `https://crates.io/crates/${crate.name}`,
          texts: [],
        };
        entry.texts.push(text);
        crates.set(key, entry);
      }
    }
  }
  for (const entry of sortedPackages([...crates.values()])) collector.add(entry);
}

/** Bundles the frontend into a temporary folder and returns Vite's licence list. */
async function viteLicenses() {
  const require = createRequire(join(DESKTOP, "package.json"));
  const { build } = await import(pathToFileURL(require.resolve("vite")).href);
  const outDir = mkdtempSync(join(tmpdir(), "blurrd-licenses-"));
  try {
    await build({
      root: DESKTOP,
      configFile: join(DESKTOP, "vite.config.ts"),
      mode: "production",
      logLevel: "error",
      build: { outDir, emptyOutDir: true, license: { fileName: "licenses.json" } },
    });
    return JSON.parse(readFileSync(join(outDir, "licenses.json"), "utf8"));
  } finally {
    rmSync(outDir, { recursive: true, force: true });
  }
}

/**
 * Installed production packages of the desktop app by `name@version`
 * (directory, author, homepage), from `pnpm licenses list`.
 */
function installedPackages() {
  // One command string: pnpm is a .cmd shim on Windows, which needs a shell.
  const result = spawnSync("pnpm licenses list --prod --json", {
    cwd: DESKTOP,
    encoding: "utf8",
    shell: true,
    maxBuffer: 64 * 1024 * 1024,
  });
  if (result.status !== 0) fail(`pnpm licenses list failed:\n${result.stderr}`);
  const packages = new Map();
  for (const list of Object.values(JSON.parse(result.stdout))) {
    for (const item of list) {
      for (const version of item.versions) {
        packages.set(`${item.name}@${version}`, {
          author: item.author ?? "",
          homepage: item.homepage ?? "",
        });
      }
    }
  }
  return packages;
}

/** Licence file names, as Vite's licence output matches them. */
const LICENSE_FILE = /^(licen[cs]e|copying)(\.(md|txt))?$/i;

/**
 * Packages whose CSS the stylesheet imports (`@import "<package>"` in
 * globals.css, compiled into the CSS asset), in Vite's licence format:
 * Vite lists only packages that end up in JavaScript chunks.
 */
function stylesheetPackages() {
  const css = readFileSync(join(DESKTOP, "src", "styles", "globals.css"), "utf8");
  return [...css.matchAll(/@import\s+"([^./][^"]*)"/g)].map(([, name]) => {
    const dir = join(DESKTOP, "node_modules", name);
    const manifest = JSON.parse(readFileSync(join(dir, "package.json"), "utf8"));
    const file = readdirSync(dir).find((entry) => LICENSE_FILE.test(entry));
    return {
      name: manifest.name,
      version: manifest.version,
      identifier: manifest.license,
      ...(file ? { text: readFileSync(join(dir, file), "utf8") } : {}),
      homepage: manifest.homepage,
    };
  });
}

/** The npm packages bundled into the frontend (scripts and stylesheet). */
async function addNpm(collector) {
  const installed = installedPackages();
  const bundled = [...(await viteLicenses()), ...stylesheetPackages()];
  const entries = bundled.map((item) => {
    const info = installed.get(`${item.name}@${item.version}`);
    const license = item.identifier ?? "";
    let text = item.text;
    let note;
    if (!text) {
      if (license !== "MIT" || !info?.author) {
        fail(`${item.name}@${item.version} ships no licence file and is not MIT with an author`);
      }
      text = MIT_TEXT.replace("{holder}", info.author);
      note = "The package ships no licence file; this is the standard MIT text it declares.";
    }
    return {
      kind: "npm",
      name: item.name,
      version: item.version,
      license,
      url: info?.homepage || item.homepage || `https://www.npmjs.com/package/${item.name}`,
      ...(note ? { note } : {}),
      textSource: { license, text },
    };
  });
  for (const { textSource, ...entry } of sortedPackages(entries)) {
    collector.add({ ...entry, texts: [collector.text(textSource.license, textSource.text)] });
  }
}

/** Packages ordered by name (case-insensitive), then version. */
function sortedPackages(entries) {
  const order = new Intl.Collator("en", { sensitivity: "base", numeric: true });
  return entries.toSorted(
    (a, b) => order.compare(a.name, b.name) || order.compare(a.version, b.version),
  );
}

async function main() {
  const check = process.argv.includes("--check");
  const collector = new Collector();
  addFfmpeg(collector);
  addOnnxRuntime(collector);
  addVcRuntime(collector);
  addModels(collector);
  addCrates(collector);
  await addNpm(collector);

  const data = { schema: SCHEMA, packages: collector.packages, texts: collector.texts };
  const outputs = [
    [DATA_FILE, `${JSON.stringify(data, null, 2)}\n`],
    [NOTICES_FILE, renderNotices(data, REPOSITORY)],
  ];
  const counts = ["binary", "model", "crate", "npm"]
    .map((kind) => `${data.packages.filter((item) => item.kind === kind).length} ${kind}`)
    .join(", ");

  if (check) {
    const stale = outputs.filter(
      ([path, content]) => !existsSync(path) || readFileSync(path, "utf8") !== content,
    );
    if (stale.length > 0) {
      fail(
        `${stale.map(([path]) => relative(ROOT, path)).join(" and ")} out of date; run \`pnpm notices\``,
      );
    }
    console.warn(`licenses: up to date (${counts}; ${data.texts.length} licence texts)`);
    return;
  }
  for (const [path, content] of outputs) writeFileSync(path, content);
  console.warn(`licenses: wrote ${counts}; ${data.texts.length} licence texts`);
}

await main();
