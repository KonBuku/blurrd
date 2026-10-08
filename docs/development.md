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
  `bin` folder (bindgen generates the FFmpeg bindings with libclang).
- [cargo-nextest](https://nexte.st) 0.9.146, [cargo-deny](https://github.com/EmbarkStudios/cargo-deny)
  0.20.2 and [typos](https://github.com/crate-ci/typos) 1.51.1, for example
  with `cargo install --locked cargo-nextest@0.9.146 cargo-deny@0.20.2 typos-cli@1.51.1`
  or from the prebuilt binaries on their release pages.
- [Node.js](https://nodejs.org) 24 LTS (`.nvmrc`) and pnpm 12.10.1
  (`npm install --global pnpm@12.10.1`; corepack is not used). The exact pnpm
  version is pinned in the `packageManager` field of `package.json`.

## Repository layout

| Path                    | Contents                                                                                     |
| ----------------------- | -------------------------------------------------------------------------------------------- |
| `apps/desktop`          | The desktop app: React frontend in `src/`, Tauri shell (crate `blurrd-app`) in `src-tauri/`. |
| `crates/blurrd-core`    | Shared types: regions, box tracks, blur plans, schemas, errors. No I/O, no `unsafe`.         |
| `crates/blurrd-blur`    | Soft-edged box blur on YUV 4:2:0 planes (I420, NV12, P010) and its quality metrics.          |
| `crates/blurrd-video`   | FFmpeg probing, decoding, seeking and the render (encode, mux); the only `unsafe` crate.     |
| `crates/blurrd-detect`  | HUD and screen detectors with their temporal models, pixel classes, morphology.              |
| `crates/blurrd-ocr`     | OCR for the verification: PP-OCRv5 detection and recognition on ONNX Runtime.                |
| `crates/blurrd-engine`  | Jobs of the CLI and the app: analysis pipeline, tracks, OCR name dictionary, cache.          |
| `crates/blurrd-cli`     | The `blurrd` command-line tool (see [cli.md](cli.md)).                                       |
| `crates/blurrd-testkit` | Development only: synthetic HUD fixtures with ground truth, quality metrics, proposals.      |
| `xtask`                 | Developer tasks, run with `cargo xtask <task>` (`cargo xtask help` lists them).              |
| `docs`                  | Developer and user documentation.                                                            |
| `scripts`               | Node scripts for repository tasks (`release.mjs`).                                           |
| `assets/brand`          | Logo and brand assets with their generator.                                                  |
| `tests/fonts`           | The OFL font of the synthetic fixtures and the pins of the fetched Noto fonts.               |

## Rust workspace

All commands run from the repository root.

```sh
cargo build --workspace                                    # debug build
cargo nextest run --workspace                              # unit and integration tests
cargo test --doc --workspace                               # doctests (nextest does not run them)
cargo bench -p blurrd-core -p blurrd-blur -p blurrd-detect # criterion benchmarks (release build)
cargo clippy --workspace --all-targets -- -D warnings      # lints, as in CI
cargo fmt --all                                            # format Rust code
cargo deny check                                           # advisories, licenses, bans, sources
typos                                                      # spelling in code and docs
```

Property tests use [proptest](https://proptest-rs.github.io/proptest/).
They run 256 cases by default; set `PROPTEST_CASES`, for example
`PROPTEST_CASES=4096 cargo nextest run`, to run more. Benchmarks use
[criterion](https://bheisler.github.io/criterion.rs/book/) and live in
`crates/*/benches`; synthetic inputs shared with the tests are in
`crates/*/tests/support`.

Tests that need real recordings are marked `#[ignore]` and read them from
the folder in `BLURRD_TEST_MEDIA`; recordings are never committed (see the
media policy in the contributing guide). Run them locally, in release
builds so the throughput checks apply, with

```sh
BLURRD_TEST_MEDIA=<folder> cargo nextest run --release --run-ignored all real_ --no-capture
```

The analysis brings recordings at other sizes to the detectors' reference
scale exactly as FFmpeg 7.1's `scale` filter does, because the detectors
were tuned and validated on such frames. The engine test
`matches_ffmpeg_on_real_recordings` compares the two byte for byte when
`BLURRD_REFERENCE_FFMPEG` names an FFmpeg 7.1 `ffmpeg` executable (it is
skipped otherwise; the FFmpeg the product links is a different version).

Tests that need a GPU or a hardware encoder (NVDEC, D3D11VA, NVENC, Media
Foundation) are ignored too, because CI runners have none; their names
start with `gpu_`. Run them with `cargo nextest run --run-ignored all gpu`.

Lints are configured once in the root `Cargo.toml` (`[workspace.lints]`) and
inherited by every crate with `[lints] workspace = true`. Production code must
not use `unwrap`, `expect`, `dbg!`, `todo!` or `println!`; tests may unwrap,
expect and print (`clippy.toml`).

### Synthetic HUD fixtures

Real recordings never enter the repository, so the detectors are tested in
CI on generated frames. `blurrd-testkit` draws HUD-like kill-feed rows,
banners, voice-chat speakers and text-chat lines with made-up names over
procedural backgrounds, in every HUD layout (16:9 at 720p to 2160p, native
4:3, pillarbox, 16:10, stretched 4:3) and colour palette, and returns the
exact ground truth of each frame: every name's glyph mask (dilated by two
reference pixels), its slot box, portraits and other text. Its `metrics`
module measures geometric leaks, the high-frequency residual, over-blur,
portrait intrusion, false positives, jitter and flicker with exactly the
mask the blur applies.

Latin text uses Barlow Condensed (SIL Open Font License 1.1), committed in
`tests/fonts/`. Names in other scripts (Hangul, Japanese, Thai, Arabic,
Devanagari) use Noto Sans fonts that are too large to commit; fetch them
once with

```sh
cargo xtask fetch-fonts   # into tests/fonts/noto/ (gitignored), SHA-256 checked
```

The download source and checksums are pinned in `tests/fonts/fonts.toml`.
Tests that need these fonts skip with a message when they are missing;
CI sets `BLURRD_REQUIRE_FONTS=1`, which turns the skip into a failure.

### Real recordings, held-out split and goldens

`tests/media/manifest.toml` lists the real recordings the local tests use:
id, file name, SHA-256 and frame count of each clip, the excerpts E1 to
E18 (frame ranges and case tags) and the split into tuning and held-out
clips. It holds no pixels and no names. Check a local copy of the
recordings against it with

```sh
cargo xtask media verify                 # the folder in BLURRD_TEST_MEDIA
cargo xtask media verify --media <DIR>
```

The ten held-out clips are never used to tune a detector, and every gate
of the architecture plan (section 11.3) must also pass on them; the split
is fixed, and a test freezes the list.

Goldens are the reviewed ground truth of the excerpts: keyframed glyph
extents of every name and the portrait rectangles, without any text
(`blurrd-golden/1`, read and written by `blurrd_testkit::golden`). They
live next to the recordings in `<BLURRD_TEST_MEDIA>/goldens/<id>.golden.json`
and are never committed. `blurrd_testkit::golden::evaluate` scores the
boxes of any frame source against a golden with the same metrics as the
synthetic fixtures, and `missing_items` is the missing-item audit: OCR text
proposals that match no golden item.

### Generated TypeScript bindings and JSON Schemas

The types in `blurrd-core` are the single source of truth for the IPC
payloads and the persisted files. Two xtask commands generate files from
them, and both outputs are committed:

```sh
cargo xtask bindings   # TypeScript types (ts-rs) into apps/desktop/src/bindings/
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
pnpm lint:typography  # only text-ui-* sizes in the desktop frontend
pnpm fmt              # format JavaScript, TypeScript, JSON, YAML, TOML, CSS and Markdown (oxfmt) and Rust
pnpm fmt:check        # check formatting without writing, as in CI
```

oxlint is configured in `.oxlintrc.json` and oxfmt in `.oxfmtrc.json`.
`CODE_OF_CONDUCT.md` is excluded from formatting because it is the verbatim
Contributor Covenant text.

## Desktop app

The desktop app lives in `apps/desktop`: a React 19 frontend built with
Vite 8, TypeScript 7 (strict) and Tailwind CSS 4 in `src/`, and the Tauri 2
shell in `src-tauri/` (the workspace crate `blurrd-app`). Run these from the
repository root:

```sh
pnpm dev              # stage the FFmpeg DLLs, start Vite and open the app with hot reload
pnpm build            # release build of the app and its NSIS installer
pnpm typecheck        # tsc --noEmit (TypeScript 7)
pnpm test             # Vitest unit tests (happy-dom)
```

`pnpm dev` runs `cargo xtask stage-dlls` first, so install the pinned
FFmpeg build once with `cargo xtask fetch-ffmpeg`. Inside `apps/desktop`,
`pnpm dev` and `pnpm build` start or build only the frontend (Vite on
port 1420), and `pnpm tauri <command>` runs the Tauri CLI. The installer is
written to `target/release/bundle/nsis/`.

`pnpm dev:mock` (inside `apps/desktop`) serves the frontend for a plain
browser: an in-memory mock backend (`src/ipc/mock`) answers the IPC calls
the Tauri shell would handle. Unit tests use the same mock; production
builds never include it. The Playwright end-to-end tests in `e2e/` run the
frontend in Chromium against this mock:

```sh
pnpm --filter @blurrd/desktop exec playwright install chromium   # once
pnpm --filter @blurrd/desktop e2e                                # starts `pnpm dev:mock` itself
```

The window has no native title bar (`decorations: false`); the frontend
draws its own with window controls. `src-tauri/capabilities/default.json`
grants the main window only the window calls of that title bar and the log
plugin's `log` command. The app writes JSON lines to
`%LOCALAPPDATA%\io.github.konbuku.blurrd\logs\blurrd.log` (debug builds
also print to stdout) and remembers the window size and position. Starting
it a second time focuses the running window.

### Design tokens and typography

`src/styles/globals.css` defines every colour, text size and easing of the
user interface as a Tailwind theme token, with the light values in `@theme`
and the dark overrides under the `.dark` class (set on `<html>` from the
system theme by `src/lib/theme.ts`). Tailwind's default palette and text
scale are switched off: use the semantic tokens (`bg-panel`,
`text-foreground-subtle`, `border-border`, ...) and the `text-ui-*` sizes,
which all follow one interface-size variable. Compose class names with
`cn` from `src/lib/utils.ts`, which knows the `text-ui-*` sizes. Lucide
icons get their 1.5 px stroke from a provider at the root.

```sh
pnpm lint:typography  # reject Tailwind's default and fixed text sizes in apps/desktop/src
```

## Git hooks

`pnpm install` installs the Git hooks with [lefthook](https://lefthook.dev)
(`lefthook.yml`):

| Hook         | Checks                                                                                                                                                 |
| ------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `pre-commit` | rustfmt, oxfmt and oxlint on the staged files, the typography check, typos                                                                             |
| `commit-msg` | commitlint (`commitlint.config.js`): Conventional Commits, header at most 72 characters, body lines at most 72 characters, scopes from CONTRIBUTING.md |
| `pre-push`   | clippy on the whole workspace with `-D warnings`                                                                                                       |

The `pre-commit` hook needs `cargo` and `typos` on `PATH`. To skip the hooks
for a single command, for example while rebasing, set `LEFTHOOK=0`.

## Continuous integration

GitHub Actions runs `.github/workflows/ci.yml` on every push to `main`, on
every pull request and on manual dispatch. All jobs must pass before a
change is merged; the installer job runs only after a merge.

| Job                | Runner  | Checks                                                                                                                                                                                                                                               |
| ------------------ | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lint`             | Ubuntu  | rustfmt, bindings and schema freshness, oxfmt, oxlint (warnings are errors), typography check, typos, gitleaks over the full history                                                                                                                 |
| `frontend`         | Ubuntu  | `pnpm typecheck` (TypeScript 7), Vitest, `vite build`, Playwright end-to-end tests in Chromium against the mock backend (report uploaded on failure)                                                                                                 |
| `rust (windows)`   | Windows | pinned FFmpeg (`cargo xtask fetch-ffmpeg`, cached), fixture fonts (`cargo xtask fetch-fonts`, cached), ONNX Runtime and OCR models (`cargo xtask fetch-ort`, `cargo xtask fetch-models`, cached), clippy with `-D warnings`, cargo-nextest, doctests |
| `deny`             | Ubuntu  | `cargo deny --locked check advisories bans licenses sources` (`deny.toml`)                                                                                                                                                                           |
| `bundle (windows)` | Windows | pushes to `main` and manual runs only, after `frontend` and `rust (windows)`: `pnpm build` (release app + NSIS installer), uploaded as the `blurrd-windows-<sha>` artifact for 14 days                                                               |

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

## ONNX Runtime and OCR models

The OCR verification runs the PP-OCRv5 text detection and recognition
models with [ONNX Runtime](https://onnxruntime.ai). Neither is committed;
both are pinned by SHA-256 and installed by xtask:

```sh
cargo xtask fetch-ort                   # ONNX Runtime + DirectML into third_party/ort/
cargo xtask fetch-models                # bundled models into models/
cargo xtask fetch-models --candidates   # also the benchmark candidates into models/candidates/
```

`third_party/ort.toml` pins two NuGet packages from nuget.org:
`Microsoft.ML.OnnxRuntime.DirectML` (ONNX Runtime with the DirectML
execution provider, MIT) and `Microsoft.AI.DirectML` (the DirectML
redistributable). `fetch-ort` verifies each package, copies
`onnxruntime.dll`, `DirectML.dll` and their license and third-party notice
files into `third_party/ort/` and stages the DLLs like `stage-dlls`. Version
1.24.4 is the last ONNX Runtime release published with the DirectML
execution provider; its C API level (24) is recorded as `api` in the pin
and must match the `api-NN` feature of the `ort` crate.

`models.toml` pins every model file by URL, size and SHA-256, together with
the conversion source: the PaddleOCR inference models (Apache-2.0) as
converted to ONNX with paddle2onnx by the RapidOCR project, at a fixed tag
of its ModelScope repository. Files marked `bundled` ship with blurrd; the
others are candidates for the model benchmark. A file whose size or
checksum differs from the pin is rejected and never replaces a good copy.

### OCR tests

`blurrd-ocr` loads ONNX Runtime from `ORT_DYLIB_PATH` and the models from
`BLURRD_MODEL_DIR` (both set in `.cargo/config.toml`). Its tests skip when
either is missing; set `BLURRD_REQUIRE_OCR=1` (as CI does) to fail
instead. CI runs on the CPU execution provider. Tests that need a
DirectX 12 GPU are ignored by default; run them locally with:

```sh
cargo nextest run -p blurrd-ocr --run-ignored all
```

The ONNX Runtime build selects its CPU kernels at run time: the CPU tests
pass under Intel SDE emulating a Pentium 4 (SSE3, no SSE4 or AVX), so
OCR needs no more than any CPU that runs Windows 11.

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
