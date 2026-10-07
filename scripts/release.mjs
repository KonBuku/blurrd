#!/usr/bin/env node
// Prepares a release commit and tag on main.
//
// Usage: pnpm release [--dry-run]
//
// git-cliff computes the next version from the Conventional Commits since the
// last tag. The script bumps the single workspace version in Cargo.toml (and
// mirrors it into the frontend package.json once that exists), regenerates
// CHANGELOG.md, then creates an unsigned commit and an annotated, unsigned tag.
// `--dry-run` only prints the next version and its changelog section. The
// script never pushes; push with `git push --follow-tags origin main`.

import { execFileSync } from "node:child_process";
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import { runGitCliff } from "git-cliff";

const FRONTEND_PACKAGE = "apps/desktop/package.json";
const dryRun = process.argv.includes("--dry-run");

/** Runs a command and returns its trimmed standard output. */
function run(command, args) {
  return execFileSync(command, args, {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  }).trim();
}

/** Runs git-cliff with the given arguments and returns its standard output. */
async function cliff(args) {
  const result = await runGitCliff(args, { stdio: ["ignore", "pipe", "inherit"] });
  return result.stdout.trim();
}

/** Replaces `version` in the `[workspace.package]` table of Cargo.toml. */
function bumpCargoVersion(version) {
  const manifest = readFileSync("Cargo.toml", "utf8");
  const bumped = manifest.replace(
    /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/,
    `$1${version}$2`,
  );
  if (bumped === manifest) {
    throw new Error("no version in the [workspace.package] table of Cargo.toml");
  }
  writeFileSync("Cargo.toml", bumped);
}

/** Mirrors the version into the frontend package.json (display only). */
function bumpFrontendVersion(version) {
  const pkg = JSON.parse(readFileSync(FRONTEND_PACKAGE, "utf8"));
  pkg.version = version;
  writeFileSync(FRONTEND_PACKAGE, `${JSON.stringify(pkg, null, 2)}\n`);
}

if (run("git", ["branch", "--show-current"]) !== "main") {
  throw new Error("releases are made from main only");
}
if (run("git", ["status", "--porcelain"]) !== "") {
  throw new Error("the working tree is not clean");
}
if (!dryRun) {
  run("git", ["pull", "--ff-only"]);
}

const tag = await cliff(["--bumped-version"]);
const version = tag.replace(/^v/, "");
console.warn(`next version: ${tag}`);

if (dryRun) {
  process.stdout.write(`${await cliff(["--unreleased", "--tag", tag])}\n`);
} else {
  const changed = ["Cargo.toml", "Cargo.lock", "CHANGELOG.md"];
  bumpCargoVersion(version);
  run("cargo", ["update", "--workspace"]);
  if (existsSync(FRONTEND_PACKAGE)) {
    bumpFrontendVersion(version);
    changed.push(FRONTEND_PACKAGE);
  }
  await cliff(["--bump", "--output", "CHANGELOG.md"]);
  run("git", ["add", ...changed]);
  run("git", ["commit", "--no-gpg-sign", "--message", `chore(release): ${tag}`]);
  run("git", ["tag", "--annotate", "--no-sign", tag, "--message", `blurrd ${tag}`]);
  console.warn(`created ${tag}; push with: git push --follow-tags origin main`);
}
