# Development

This guide explains how to build, test and check blurrd locally. The
[contributing guide](../CONTRIBUTING.md) covers the ground rules, the media
policy and the commit conventions.

## Prerequisites

- Windows 11 x64 with the MSVC build tools and the Windows 11 SDK
  (Visual Studio 2022).
- Rust via [rustup](https://rustup.rs). `rust-toolchain.toml` pins the
  toolchain (1.99.0 with rustfmt and clippy); rustup installs it on the first
  `cargo` command inside the repository.
- [LLVM](https://llvm.org) 21 or newer, with `LIBCLANG_PATH` pointing to its
  `bin` folder (needed by bindgen once the FFmpeg bindings are added).
- [cargo-nextest](https://nexte.st) 0.9.146, [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
  0.20.2 and [typos](https://github.com/crate-ci/typos) 1.51.1, for example
  with `cargo install --locked cargo-nextest@0.9.146 cargo-deny@0.20.2 typos-cli@1.51.1`
  or from the prebuilt binaries on their release pages.
- [Node.js](https://nodejs.org) 24 LTS (`.nvmrc`) and pnpm 12.10.1
  (`npm install --global pnpm@12.10.1`; corepack is not used). The exact pnpm
  version is pinned in the `packageManager` field of `package.json`.

## Repository layout

| Path                 | Contents                                                                             |
| -------------------- | ------------------------------------------------------------------------------------ |
| `crates/blurrd-core` | Shared types: regions, box tracks, blur plans, schemas, errors. No I/O, no `unsafe`. |
| `crates/blurrd-blur` | Soft-edged box blur on YUV 4:2:0 planes (I420, NV12, P010) and its quality metrics.  |
| `xtask`              | Developer tasks, run with `cargo xtask <task>` (`cargo xtask help` lists them).      |
| `docs`               | Developer and user documentation.                                                    |
| `scripts`            | Node scripts for repository tasks (`release.mjs`).                                   |
| `assets/brand`       | Logo and brand assets with their generator.                                          |

## Rust workspace

All commands run from the repository root.

```sh
cargo build --workspace                               # debug build
cargo nextest run --workspace                         # unit and integration tests
cargo test --doc --workspace                          # doctests (nextest does not run them)
cargo bench -p blurrd-core -p blurrd-blur             # criterion benchmarks (release build)
cargo clippy --workspace --all-targets -- -D warnings # lints, as in CI
cargo fmt --all                                       # format Rust code
cargo deny check                                      # advisories, licenses, bans, sources
typos                                                 # spelling in code and docs
```

Property tests use [proptest](https://proptest-rs.github.io/proptest/).
They run 256 cases by default; set `PROPTEST_CASES`, for example
`PROPTEST_CASES=4096 cargo nextest run`, to run more. Benchmarks use
[criterion](https://bheisler.github.io/criterion.rs/book/) and live in
`crates/*/benches`; synthetic inputs shared with the tests are in
`crates/*/tests/support`.

Lints are configured once in the root `Cargo.toml` (`[workspace.lints]`) and
inherited by every crate with `[lints] workspace = true`. Production code must
not use `unwrap`, `expect`, `dbg!`, `todo!` or `println!`; tests may unwrap,
expect and print (`clippy.toml`).

### Generated TypeScript bindings and JSON Schemas

The types in `blurrd-core` are the single source of truth for the IPC
payloads and the persisted files. Two xtask commands generate files from
them, and both outputs are committed:

```sh
cargo xtask bindings   # TypeScript types (ts-rs) into crates/blurrd-core/bindings/
cargo xtask schema     # JSON Schemas of project, settings, presets and verify reports into docs/schema/
```

Run both after changing a type that derives `TS` or `JsonSchema` and commit
the result together with the change. The output folder of the bindings is
set by `TS_RS_EXPORT_DIR` in `.cargo/config.toml`, where `TS_RS_LARGE_INT`
also maps 64-bit integers to `number`. The JSON Schemas are derived only
with the `schema` feature of `blurrd-core`, which xtask enables. The
generated files are excluded from oxfmt and kept exactly as written; CI
regenerates them and fails if anything differs.

### Build profiles

| Profile    | Use                                                                                                                     |
| ---------- | ----------------------------------------------------------------------------------------------------------------------- |
| `dev`      | Day-to-day builds. Dependencies are optimised (`opt-level = 2`) so image and OCR code stays usable.                     |
| `release`  | Desktop app releases. Panics unwind, because the app catches panics on job threads and reports them as internal errors. |
| `dist-cli` | Distributed `blurrd` CLI binaries (`cargo build --profile dist-cli`). Inherits `release` but aborts on panic.           |

## pnpm workspace

The repository root is a pnpm workspace (`pnpm-workspace.yaml`); the desktop
frontend joins it as `apps/desktop`. The root `package.json` pins the
repository tooling to exact versions.

```sh
pnpm install          # install the tooling
pnpm lint             # oxlint, warnings are errors
pnpm lint:fix         # apply oxlint's safe fixes
pnpm fmt              # format JavaScript, TypeScript, JSON, YAML, TOML, CSS and Markdown (oxfmt) and Rust
pnpm fmt:check        # check formatting without writing, as in CI
```

oxlint is configured in `.oxlintrc.json` and oxfmt in `.oxfmtrc.json`.
`CODE_OF_CONDUCT.md` is excluded from formatting because it is the verbatim
Contributor Covenant text.

## Git hooks

`pnpm install` installs the Git hooks with [lefthook](https://lefthook.dev)
(`lefthook.yml`):

| Hook         | Checks                                                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pre-commit` | rustfmt, oxfmt and oxlint on the staged files, typos                                                                                                   |
| `commit-msg` | commitlint (`commitlint.config.js`): Conventional Commits, header at most 72 characters, body lines at most 72 characters, scopes from CONTRIBUTING.md |
| `pre-push`   | clippy on the whole workspace with `-D warnings`                                                                                                       |

The `pre-commit` hook needs `cargo` and `typos` on `PATH`. To skip the hooks
for a single command, for example while rebasing, set `LEFTHOOK=0`.

## Continuous integration

GitHub Actions runs `.github/workflows/ci.yml` on every push to `main`, on
every pull request and on manual dispatch. All jobs must pass before a
change is merged.

| Job              | Runner  | Checks                                                                                                             |
| ---------------- | ------- | ------------------------------------------------------------------------------------------------------------------ |
| `lint`           | Ubuntu  | rustfmt, bindings and schema freshness, oxfmt, oxlint (warnings are errors), typos, gitleaks over the full history |
| `rust (windows)` | Windows | pinned FFmpeg (`cargo xtask fetch-ffmpeg`, cached), clippy with `-D warnings`, cargo-nextest, doctests             |
| `deny`           | Ubuntu  | `cargo deny --locked check advisories bans licenses sources` (`deny.toml`)                                         |

`.github/workflows/ffmpeg.yml` builds the FFmpeg libraries blurrd ships
(see [FFmpeg](#ffmpeg)). Two more workflows guard pull requests and the
workflows themselves:

- `.github/workflows/pr-title.yml` checks the pull request title with
  commitlint, because the title becomes the squashed commit header on
  `main`. Pull requests opened by Dependabot are skipped. Check a title
  locally with `echo "feat(detect): ..." | pnpm exec commitlint`.
- `.github/workflows/zizmor.yml` runs [zizmor](https://docs.zizmor.sh)
  1.30.1 on every change under `.github/workflows/`. Run it locally with
  `uvx zizmor@1.30.1 .github` (set `GH_TOKEN` for the online audits).

The jobs run the same commands as the sections above, so a green local run
predicts a green CI run. To reproduce the secret scan locally, install
[gitleaks](https://github.com/gitleaks/gitleaks) 8.30.1 and run
`gitleaks git --redact --verbose .` in the repository root.

Pinning rules:

- Every action is pinned to a full commit SHA with the release tag as a
  comment (`uses: owner/action@<sha> # vX.Y.Z`). Dependabot updates the SHA
  and the comment together.
- The Rust toolchain comes from `rust-toolchain.toml`: CI runs
  `rustup toolchain install` with the rustup that ships on the runner, so CI
  and local builds always use the same toolchain and components.
- cargo-nextest, cargo-deny and gitleaks are installed by exact version from
  the `env` block of `ci.yml`. gitleaks is downloaded from its GitHub release
  and checked against a pinned SHA-256; when you bump `GITLEAKS_VERSION`,
  update `GITLEAKS_SHA256` from the `gitleaks_<version>_checksums.txt` file
  of the same release.
- Workflows request `contents: read` only; a job asks for more permissions
  only when it needs them.

Dependabot (`.github/dependabot.yml`) opens grouped weekly updates for
Cargo, npm and GitHub Actions, one week after a release at the earliest.
Its commits use `build(deps): ...` for Cargo and npm and `ci(deps): ...`
for actions, so `cliff.toml` lists the crate and package updates under
"Dependencies". Shorten a generated title to 72 characters when squashing.
Review the Rust `tauri*` and the `@tauri-apps/*` updates together so both
stay on the same minor version.

## FFmpeg

blurrd links its own minimal FFmpeg build dynamically: FFmpeg 9.0.2 under
the LGPL-2.1-or-later, with the codec, format, scaling and resampling
libraries, the hardware codecs (NVENC/NVDEC, AMF, Intel QSV through oneVPL,
Media Foundation, D3D11VA), dav1d for AV1 decoding and zlib. It contains no
GPL or non-free parts and no other external libraries. The FFmpeg on your
`PATH`, if any, is never linked or shipped.

The build is defined in `third_party/ffmpeg-build/`:

| File                       | Purpose                                                                         |
| -------------------------- | ------------------------------------------------------------------------------- |
| `vcpkg.json`               | the `ffmpeg` port (version 9.0.2) and its features                              |
| `vcpkg-configuration.json` | the vcpkg baseline commit that pins every port version                          |
| `triplets/`                | `x64-windows-blurrd`: shared libraries, dynamic CRT, release only               |
| `build.ps1`                | builds, checks the configuration, packages the binaries and the matching source |

`.github/workflows/ffmpeg.yml` runs `build.ps1` on every change to that
folder. The check fails the build if the configure line contains
`--enable-gpl`, `--enable-nonfree`, `--enable-version3` or
`--enable-libopenh264`, if `ffprobe -L` does not report the LGPL 2.1 or
later, or if a required encoder, decoder or hardware acceleration is
missing. A manual run with `revision` set to N and `publish` checked uploads
three files to a new release `deps-ffmpeg-9.0.2-rN`:

- `ffmpeg-9.0.2-rN-win64-lgpl-shared.zip`: `bin/` (DLLs and `ffprobe.exe`),
  `include/`, `lib/` (import libraries), `licenses/` (the copyright file of
  every vcpkg package) and `BUILDINFO.txt` (configure line, package
  versions);
- `ffmpeg-9.0.2-rN-source.zip`: the FFmpeg release tarball, the vcpkg port
  files with their patches and the build definition, which is the
  corresponding source the LGPL requires us to offer;
- `SHA256SUMS.txt`.

An existing release is never replaced; dispatch the next revision instead.
To reproduce a build locally, install PowerShell 7, clone vcpkg at the
baseline commit and run the script from a short path without spaces:

```powershell
git clone https://github.com/microsoft/vcpkg C:\vcpkg-blurrd
git -C C:\vcpkg-blurrd checkout <baseline from vcpkg-configuration.json>
C:\vcpkg-blurrd\bootstrap-vcpkg.bat -disableMetrics
pwsh third_party/ffmpeg-build/build.ps1 -VcpkgRoot C:\vcpkg-blurrd -OutDir C:\ffb -Revision 0
```

To change the build (another FFmpeg version, a feature), edit the manifest
or move the baseline, let the workflow build it, publish a new revision and
pin it.

### Installing the pinned build

`third_party/ffmpeg.toml` pins the release tag, the binary archive and its
SHA-256 (and the matching source archive). Install it once after cloning
and again whenever the pin changes:

```sh
cargo xtask fetch-ffmpeg                       # download, verify, unpack, stage DLLs
cargo xtask fetch-ffmpeg --archive <file.zip>  # install a local copy of the pinned archive
cargo xtask stage-dlls                         # copy the DLLs into target/ again
```

`fetch-ffmpeg` unpacks the archive into `third_party/ffmpeg/` (gitignored),
which `.cargo/config.toml` passes to the bindings as `FFMPEG_DIR`. Any
archive whose checksum differs from the pin is rejected. While the
repository is private, the download authenticates with `GH_TOKEN` or
`GITHUB_TOKEN` if set, otherwise with the login of the
[GitHub CLI](https://cli.github.com) (`gh auth login`); the token is passed
to curl on standard input, never on the command line.

Windows loads the FFmpeg DLLs from the folder of the executable, so
`fetch-ffmpeg` also copies them into `target/<profile>/` and
`target/<profile>/deps/` (where the test binaries live) for `debug`,
`release` and every other profile folder that exists. Run
`cargo xtask stage-dlls` after `cargo clean` or after building a new
profile; a test binary that exits with `STATUS_DLL_NOT_FOUND`
(`0xc0000135`) is missing them.

## Changelog and releases

`CHANGELOG.md` and the release notes are generated from the commit history by
[git-cliff](https://git-cliff.org) (`cliff.toml`); never edit the changelog by
hand.

```sh
pnpm changelog            # preview the unreleased section
pnpm release --dry-run    # show the next version and its changelog section
pnpm release              # bump, regenerate CHANGELOG.md, commit and tag
```

`pnpm release` runs on a clean `main` only. It bumps the single version in
`[workspace.package]` of `Cargo.toml`, syncs `Cargo.lock`, regenerates
`CHANGELOG.md`, and creates an unsigned commit `chore(release): vX.Y.Z` with
an annotated, unsigned tag `vX.Y.Z`. It does not push; run
`git push --follow-tags origin main` afterwards.
