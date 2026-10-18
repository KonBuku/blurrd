# Development

This guide explains how to build, test and check blurrd locally. The
[contributing guide](../CONTRIBUTING.md) covers the ground rules, the media
policy and the commit conventions, [architecture.md](architecture.md) how
the crates and the jobs fit together, and [privacy.md](privacy.md) what
the verification promises and which cases are verified.

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
- [cargo-about](https://github.com/EmbarkStudios/cargo-about) 0.9.2 for the
  third-party notices: `cargo install --locked cargo-about@0.9.2 --features cli`
  (the binary needs the `cli` feature) or the prebuilt binary.
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

Run the command-line tool from the checkout with `cargo run`, which also
points it at the ONNX Runtime and the models that xtask installed (see
[cli.md](cli.md#running-the-tool)):

```sh
cargo run --release -p blurrd-cli -- probe <recording>
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

`/?gallery` on the mock server shows the component gallery: every token
and component in one page. `e2e/gallery.spec.ts` checks that every control
in it shows a visible focus ring, and compares light and dark screenshots
of the gallery, the window shell and the floating layers with the
baselines in `e2e/gallery.spec.ts-snapshots`. The baselines are rendered on
Windows (Segoe UI) and the screenshot tests skip themselves elsewhere.
After an intended visual change, update them on Windows and review the
new images before committing:

```sh
pnpm --filter @blurrd/desktop e2e --grep "@screenshot" --update-snapshots=all
```

(`=all` rewrites every baseline; the default only rewrites images that
differ by more than the 1 % tolerance, so small changes would be missed.)

The window has no native title bar (`decorations: false`); the frontend
draws its own with window controls. `src-tauri/capabilities/default.json`
grants the main window the app's engine commands (declared in
`src-tauri/build.rs`, so each one needs a permission), the window calls of
that title bar, the log
plugin's `log` command, reading and writing the settings store that the
Rust side opens at start (the frontend cannot open other store files), and
the open dialog for adding recordings. The app writes JSON lines to
`%LOCALAPPDATA%\io.github.konbuku.blurrd\logs\blurrd.log` (debug builds
also print to stdout) and remembers the window size and position. Starting
it a second time focuses the running window and hands the files on the
second command line to it (event `second-instance`), which adds them to
the queue.

### Engine commands

`src-tauri/src/commands.rs` exposes the engine to the frontend (plan 9.1);
the typed wrappers are `src/ipc/commands.ts`, `jobs.ts` and `presets.ts`,
and their payloads are generated bindings. `src/ipc/contract.test.ts`
checks that every command a wrapper invokes is declared in `build.rs`,
registered in `src-tauri/src/lib.rs`, granted in the capability and
answered by the mock backend. The commands take only paths
the user chose: the open and save dialogs and drops onto the window add
them to the asset protocol scope, and `src-tauri/src/paths.rs` refuses
anything outside it. Projects (`projects.rs`) are read from and written to
the engine's cache folder on every call, so a finished analysis shows at
once and edits are checked against the project revision.

Jobs (`jobs.rs`) wait in one queue and run one at a time on its worker
thread. Each reports over the `Channel<JobEvent>` passed with its command;
progress reaches the channel in frame order and at most 20 times per
second, whatever engine thread reports it. Pausing holds the queue and the
running job at its next progress report; cancelling ends a waiting job at
once and a running one at the engine's next frame. Closing the app cancels
every job, so no partial output is left behind. The engine's `tracing`
events go to the app log through the `log` facade.

The `blurrd://` scheme (`src-tauri/src/scheme.rs`, `http://blurrd.localhost/`
on Windows) serves the timeline thumbnails of an analysis, the queue's
poster frames (`poster.jpg?path=<file>`, one frame a tenth into a chosen
video, decoded on request before any analysis; `src-tauri/src/poster.rs`),
the Exact patches of a frame (`exact/<project>/<frame>.json` lists them,
`exact/<project>/<frame>/<roi>.jpg` is one) and the crops of a report's
leaks. An Exact patch is the frame blurred by the export's own blur
(`src-tauri/src/exact.rs`): the request carries the inspector's blur and
enabled regions (`settings=` query, `ExactSettings` as JSON), overlapping
padded boxes merge into one patch, and a frame seeker stays open on the
last project so that stepping through frames is answered from decoded
frames.

Videos of every size and their preview proxies play from the loopback
media server (`src-tauri/src/media_server.rs`): it listens on 127.0.0.1
on a random port, answers only URLs with the random token of this app
start and the id of an opened video or proxy (until `close_project`),
and answers range requests straight from the file in 256 KiB writes. In
WebView2 with GPU decoding it read 6 to 8 times faster than the asset
protocol and kept seek tails short (90th percentile 59 ms against
103-107 ms, slowest seek 63 ms against about 200 ms; the asset
protocol's median far seek was slightly lower, 22-23 ms against
31 ms), and it did not slow down while the window's thread was busy.
Only when the server cannot start do videos play
through the asset protocol (`src-tauri/src/media.rs` logs it).

### Window shell

`src/features/shell/AppShell.tsx` lays out the main window: the title bar,
the queue sidebar on the window shell colour, the preview frame and the
inspector frame. The 4 px gaps between them are resize handles (drag, or
focus them and use the arrow keys, Home and End; a double-click resets the
width). `src/lib/layout.ts` clamps the preferred widths to the window
(queue 264 px to 40 % of the window, inspector 300 to 420 px, preview at
least 480 px) and turns the inspector into an overlay sheet below 1180 px.
Ctrl+B shows or hides the queue, Ctrl+Alt+B the inspector; the shortcut
table is `src/lib/shortcuts.ts`. Panel visibility and widths are kept in
the WebView's local storage (`src/store/ui.ts`), not in the settings file.

The settings view (gear button or Ctrl+, in the title bar; Escape closes
it) replaces the preview and inspector. Appearance (theme, interface size,
reduce motion) is applied by `src/lib/appearance.ts` and saved by
`src/store/settings.ts` to `settings.json` (schema `blurrd-settings/1`) in
`%APPDATA%\io.github.konbuku.blurrd` through the store plugin. A fresh
install is dark (Settings › Appearance offers System, Light and Dark; a
stored choice wins). No start flashes white: the Rust side creates the
window (`create: false` in `tauri.conf.json`) with the window shell colour
of the stored theme as its and the WebView's background, and
`public/boot.css` and `public/boot.js` paint the cached theme and the
brand mark as a loading screen before the app's bundle has loaded. The
window hands its theme to the page (`window.blurrdBootTheme`, an
initialization script), so the first paint matches the window even when
the copy of the appearance in local storage is stale; the copy is used
outside the app (dev server, mock). A "System" window stays hidden until
it has the colour of the Windows theme. index.html has no inline `<style>` or `<script>`: Tauri would
add a nonce to the CSP, which disables the `'unsafe-inline'` that React's
style attributes need (`src/styles/first-paint.test.ts` keeps the colours
and this in sync). A settings file of another schema major is read but
never overwritten. `e2e/settings.spec.ts` covers the dark default, theme
switching (system, light, dark, title bar menu), persistence across
reloads, interface size, reduce motion, focus handling and an axe scan of
each page in both themes; tests that check both themes pick one with
`chooseTheme` (`e2e/fixtures.ts`).

The pages are General, Appearance, Processing, Output defaults,
Detection, Verification, Shortcuts, Logs & privacy and About. The
frontend writes `appearance` and `general` itself; `processing`,
`defaults` and `privacy` go through commands (`get_settings`,
`save_processing`, `save_defaults`, `save_privacy`), so Rust checks every
value (`src-tauri/src/settings.rs`) before the store writes it. Processing
offers only what the engine reads at start (GPU on or off, the OCR GPU)
and "Measure again" (`refresh_capabilities` clears `caps\`). The stored
defaults are the settings of files added later; changing them on a page
keeps the values of files already queued, and the inspector still
overrides them per file. The output folder of the defaults is allowed
again at start (the user chose it in an earlier session). Logs & privacy
sets the log level (applied at once), opens the log folder
(`open_log_dir`) and clears cached analyses of recordings that are not
queued (`get_cache_info`, `clear_cache`). `e2e/settings-pages.spec.ts`
covers navigation, axe in both themes, the 1024x680 window, keyboard
use, persistence across reloads and the export folder in the inspector.

The command palette (`src/features/command`, Ctrl+K or the search button
in the title bar) lists the actions of the mounted features. A feature
offers its commands with `useCommandSource(source, commands)` while it is
on screen (the preview's only while a recording is shown), so the palette
never lists an action without a target. Search requires every typed word
in the label, group or keywords (`commandScore`); a chosen command runs
after the palette has closed and returned the focus. `e2e/command.spec.ts`
covers the keyboard flow, the file dialog, the preview commands and axe.

About (`src/features/about`) is a dialog from the palette and the page
Settings › About: build version and commit (Vite `define`s
`import.meta.env.APP_VERSION` / `APP_COMMIT`), the FFmpeg version the
backend reports, the FFmpeg source archive, project links (the opener
capability allows only `https://github.com/KonBuku/blurrd` URLs) and the
trademark statements. Open-source licenses lists the generated
`licenses.json` (see [Third-party notices](#third-party-notices)), loaded
as its own chunk on first use; `e2e/about.spec.ts` covers search, licence
texts, links, focus return and axe in both themes.

`e2e/layout.spec.ts` checks the layout from 1024x680 to 2560x1440 (no
overflow, limits respected, sheet below the breakpoint), resizing by drag
and keyboard, and runs an axe accessibility scan of the shell and the sheet
in both themes. Native window behaviour cannot be tested in a browser;
check it by hand in `pnpm dev` after changing the title bar: maximize and
restore with the button and by double-clicking the title bar, drag the
title bar to a screen edge (snap) and to the top (maximize), Win+arrow
keys, Win+Z (snap layouts), and that a theme chosen in Settings survives
a restart.

### Queue and import

Recordings join the queue from the + button and the queue menu (file and
folder dialogs, Ctrl+O and Ctrl+Shift+O), from files dropped anywhere on
the window (Tauri drag and drop events; an accent veil says how many items
a drop adds) and from a second start. `src/features/import/import.tsx`
shows each new path as "Reading…" at once, asks the backend to probe the
paths (`probe_media` in `src/ipc/commands.ts`; folders expand to the
videos inside them) and lists every file it cannot use in one toast.
Paths already in the queue are skipped, compared case-insensitively.

`src/store/queue.ts` holds the items, the selection and the filter;
`src/lib/queue-status.ts` is the status model of design 8.1 (queued,
reading, analyzing, ready, exporting, verifying, done, done with warnings,
unverified, failed, canceled) with the status text, and
`src/features/queue/StatusGlyph.tsx` the glyph of each status; a status is
never shown by colour alone. The list (`QueueList.tsx`) is a virtualized
multi-select listbox: only the visible rows are in the DOM, so whole
recording folders stay smooth. Click, Ctrl+click and Shift+click select;
arrows, Page Up/Down, Home and End move (Shift extends), Ctrl+A selects
all, Delete removes; the context menu copies paths and removes. A filter
field appears above eight items. `e2e/queue.spec.ts` covers the import
paths, the keyboard and mouse selection, the context menu, the filter, an
axe scan in both themes, and drops 200 files to check that the list keeps
fewer than 30 rows in the DOM and scrolls at the idle page's frame rate
with no long tasks.

While the queue is empty, the preview frame shows the empty state
(`src/features/empty/EmptyState.tsx`): Browse files and Add folder, the
accepted formats, the GPU card and a dismissible getting-started checklist
(dismissal kept in local storage). The GPU card
(`src/features/gpu/GpuStatusCard.tsx`, also in the queue footer) renders
at once with "Probing GPU…" and polls `get_capabilities` through TanStack
Query (`src/lib/query.ts`) every 500 ms until the backend reports the
probe as ready; `src/lib/gpu.ts` turns the capabilities into the GPU, CPU
fallback or no-encoder state and the capability chips.

`src/store/session.ts` saves the queue (path, resting status, output)
300 ms after every change through `save_session` and restores it with
`get_session` on start, then probes the restored files again; a file that
is gone is marked failed ("File not found"). Running jobs are saved as
their resting status (an interrupted analysis is queued again, an
interrupted export leaves the file ready). Settings › General › "Restore
the queue" switches this off; the saved queue is then cleared.
`e2e/session.spec.ts` covers restore after a reload, every saved status
including UNVERIFIED, the setting, the GPU card from "probing" to the
results with the first paint before the probe ends, the empty state and
axe scans.

The mock backend (`src/ipc/mock`) has a small fake file system for this:
`window.blurrdMock.dialogResult` is what the next dialog returns,
`folders` maps folder paths to their files, `drop()`, `dragEnter()`,
`dragLeave()` and `secondInstance()` emit the events of the real shell, and
`probeError` makes the probe fail. It reports an RTX 40 machine whose
probe takes `capabilitiesDelayMs` (800 ms; session storage
`blurrd.mock.capabilitiesDelayMs` sets it before the page loads) and keeps
the saved queue in session storage (`sessionFile()`), so a reload behaves
like a restart.

### Preview

Selecting a recording in the queue opens its project (`open_project`) and
shows it in the preview frame (`src/features/preview`); removing it from
the queue closes the project (`close_project`,
`src/store/project-lifetime.ts`). The `<video>`
element plays the source; a WebGL 2 canvas on top of it
(`gl/blur-renderer.ts`) takes every presented frame
(`requestVideoFrameCallback`, and `seeked` while paused), looks up its
boxes in `src/lib/interval-index.ts` (the per-frame boxes of the export,
built from `get_tracks`: track boxes with the bound Suppress/Adjust edits,
then the Add edits; Manual boxes ignore the region switches) and blurs
them with the steps of `CpuBlur`: block means with the per-block noise,
bilinear upsample and a separable gaussian with a replicated border, hard
blocks or a solid fill, mixed in through the rounded-rectangle mask, boxes
composited in order. `src/lib/blur-geometry.ts` ports `PixelParams`,
`prepare_boxes`, the block grid, the taps, the kernel and the noise with
32-bit float rounding; its unit tests compare against values printed by
the Rust crate. Because the browser hands out RGB frames, luma is blurred
at full resolution and chroma (B−Y, R−Y) on its own half-resolution grid,
mixed with the max-pooled chroma mask like the 4:2:0 export. Before, After
and Split (a draggable, keyboard-operable divider) are drawn by the same
canvas. The canvas, the video element and the box overlay share one box
on the stage (`src/lib/stage.ts`): Fit or 25–400 %, Ctrl+wheel zooms
around the pointer, the wheel and a drag pan when the frame is larger
than the stage. Without WebGL 2 the video shows unblurred and the header
says so.

When WebView2 cannot decode the source (HEVC without the HEVC Video
Extensions, AV1 without a decoder), the stage plays the H.264 proxy from
the analysis instead (`source.ts`: a codec check before loading and a
fallback on a load or decode error); the canvas still renders at source
size, so boxes and frame numbers stay in source pixels. Without a proxy
the stage explains how to get one. "Exact" (header) asks the backend for
its own blur of the HUD regions around a paused frame
(`blurrd://exact/<project>/<frame>.json` and the JPEG patches,
`src/ipc/media.ts`) and draws those patches over the WebGL result.

"Boxes" (header) shows the box overlay (`BoxOverlay.tsx`,
`overlay-model.ts`): every box the export blurs on the presented frame
with a 1 px stroke and a 10 % fill in its region colour, open leaks of the
last verification with a dashed destructive stroke (accepted ones and
names outside the regions dashed in the warning colour) and edits that no
longer match a detection dashed and transparent. Each item is a button
named like its tooltip ("Round banner · player name · frames 70–160");
clicking selects it. A halo in the background colour keeps the strokes
visible on any footage. Items are positioned from source rectangles with
the stage box, so they stay within one source pixel of the video at
every zoom (`e2e/overlay.spec.ts` measures 25–400 %).

Editing (`src/features/editing`, `src/lib/edits.ts`, `src/store/edits.ts`):
M (or the square-plus button in the transport) starts the manual box
tool; dragging a rectangle on the stage adds a Manual box (an `Add` edit)
over one second either side of the frame where the tool started, or
over the in and out points set with I and O while the tool is active.
A selected manual or detector box gets eight resize handles and can be
dragged; Alt+arrows move it by one source pixel, Alt+Shift+arrows resize
it. Moving a detector box adds an `Adjust` edit over its track (changed
again on the next move, removed when the box is back where the detector
put it); Delete hides a detector box (`Suppress`), removes a manual box
or another edit, and on a hidden box (dashed, "hidden box") shows it
again. With a manual box selected, I and O set its frames (O toggles the
overlay otherwise). Esc leaves the tool, then clears the selection.

The edit store owns the override list of the shown project: every edit
is one zundo history step (Ctrl+Z, Ctrl+Shift+Z / Ctrl+Y, the transport
buttons; 500 steps per project). `OverrideSync` sends the full list with
the revision it was based on (`set_overrides(project_id, base_rev,
overrides)`, debounced 150 ms, one request at a time) and updates the
cached tracks and the project's unverified ranges with the answer; the
transport shows "Saving…", "Saved" or "Not saved" with Retry. A
`StaleRevision` answer (another writer or a finished re-analysis) reloads
the stored edits, drops the local history and says so in a toast; a new
stored revision while nothing is being saved is taken over the same
way. After a re-analysis, `Suppress` and `Adjust` edits rebind by IoU;
those without a match stay in the list, dashed and inactive.
`src/features/editing/override-sync.test.ts` makes 100 seeded random
edits against the mock backend, undoes all of them and checks that the
stored list, its bindings and the boxes of every frame are identical to
the start; `e2e/editing.spec.ts` drives the tool, the handles, undo and
redo, a stale revision and a re-analysis in the browser.

The timeline under the stage (`src/features/timeline`) shows a ruler,
one density lane per region with boxes (`get_timeline_density`, 2,048
buckets resampled to pixel columns, at least 2 px for a single frame),
warning-outlined spans where a region fell back to its safe slot boxes
(with a warning button on the lane label), the Verify lane (green = OCR
clean, hatched = not verified yet or changed since the report, red = open
leak), leak markers (buttons: jump to the leak and select its box), the
optional thumbnail strip (Settings › Appearance › Timeline thumbnails)
and the playhead with its timecode. Click or drag scrubs to the frame
under the pointer, Ctrl+wheel zooms around it (down to 30 frames), the
wheel pans, and the view follows the playhead. The track is a
`role="slider"`: arrows step one frame, Page Up/Down one second, Home/End
jump to the ends. Hovering shows the nearest thumbnail, the timecode, the
frame number and the boxes per region.

The transport under the stage (`Transport.tsx`, `playback.ts`) steps
frames, jumps to the ends, plays and pauses, shows the timecode and the
duration or (after a click) the 0-based decoded frame index, sets the
speed (0.25–2×) and mutes the preview audio. Every move goes through the
playback controller: seeks aim at the middle of a frame's display
interval from `pts.bin`, and the shown frame number is always taken from
the frame the browser presented, never from the requested time. J and L
shuttle backwards and forwards, faster with every press (1, 2, 4×; the
video element cannot play backwards, so reverse steps on a timer).

Preview shortcuts (`src/lib/shortcuts.ts`, `usePreviewShortcuts.ts`,
listed in Settings › Shortcuts): Space / K play or pause, J / L shuttle,
← / → one frame, Shift+← / → one second, Home / End, B / A / S for
Before / After / Split, O for the box overlay, N / Shift+N for the next
or previous detection, user box or unfixed leak, Ctrl+= / Ctrl+- /
Ctrl+0 to zoom, and the editing keys above. Ctrl+Z / Ctrl+Y in a text
field stay the field's own undo and redo, Delete in the queue list
removes recordings. They are ignored while a dialog or menu is open, in text
fields, and for keys the focused control uses itself (arrows in the
queue list or a slider, Space on a button).

Per-frame work during playback is kept small: only the playhead, the
slider value, the position text and the canvas subscribe to the frame;
the overlay re-renders only when its items change. `e2e/transport.spec.ts`
checks 20 seeded random seeks through the timeline, the keys, the slider
and the transport (shown number = frame on the canvas = index decoded in
the video = frame of the backend's Exact patch) and plays the fixture:
at least 57 presented frames per second, no long tasks.

Preview blur parity: `e2e/blur-parity.spec.ts` blurs reference frames
with the WebGL renderer and compares them with the frames `CpuBlur`
produced. Write the reference frames first (synthetic frames; `--i420`
adds a raw `yuv420p` frame of your own footage for a local check):

```sh
cargo run -p blurrd-blur --example reference_frames   # writes target/blur-reference
pnpm --filter @blurrd/desktop e2e blur-parity
```

Tolerance, 8-bit RGB per channel over the padded boxes: mean ≤ 0.5,
99th percentile ≤ 2, maximum ≤ 16 (the maximum occurs only where the YUV
result lies outside the RGB gamut and the browser's RGB input was already
clipped); pixels outside the boxes are identical. The test skips without
reference frames unless `BLURRD_REQUIRE_BLUR_REFERENCE` is set (CI).

On real footage the difference is larger. Measured against the engine
itself (the CLI's lossless FFV1 render of a 16 s 1080p60 recording with
kill feed, banner and chat boxes; 12 frames with 2 to 5 boxes, 213,108
values) in headless Edge 155, the engine of WebView2, with ANGLE on
Direct3D 11 and on SwiftShader alike, two independent measurements:
mean 0.51 to 0.64 (at most 0.93 per frame), 99th percentile 3 to 4 (at
most 8 per frame), maximum 11 to 19; pixels outside the boxes
identical. In the same setup, 20 seeded random seeks
on that recording (hardware and software decoding) presented frame N
every time: the shown number, the frame of `requestVideoFrameCallback`
and the picture (closest to the engine's decoded frame N, not N ± 1)
agreed.

The mock backend plays synthetic fixture videos for every opened file
(`src/ipc/mock/fixture-media.ts`): name plates that appear and slide like
kill-feed rows, a banner and a chat line, an alignment target and the
frame's own index as a row of black and white squares, so tests can read
which frame is on the canvas. `scripts/make-fixture-media.mjs` encodes
them with FFmpeg (libvpx-vp9 and libx265) into `e2e/fixtures/media`
(ignored by git) before the end-to-end tests run; run `pnpm fixtures`
inside `apps/desktop` before `pnpm dev:mock`. Paths containing "hevc"
open the HEVC fixture with a VP9 proxy, paths containing "fresh" are not
analysed yet. The mock keeps the edits per opened file
(`src/ipc/mock/edits.ts`): `set_overrides` rejects a stale `base_rev`
and invalid edits, stores a new revision and marks the frames of changed
edits unverified; `window.blurrdMock.projects.touch(path)` and
`.reanalyze(path, dy)` simulate another writer and a re-analysis. Without FFmpeg the preview tests skip locally and fail in CI.

### Inspector

The inspector (`src/features/inspector`) edits the job settings of the
queued files (`src/store/job-settings.ts`, `src/lib/job-settings.ts`):
regions and box fit for `start_analysis`, blur and export settings for
`start_render`, and the verification settings (OCR on or off and the
coverage). The inspector offers only what the engine reads. Every file
has its own settings once it is changed; files without own settings, and
files added later, use the defaults for new files. "Apply to" switches
between the selected files (or the defaults when nothing is selected) and
every file; a change writes only the field that changed, so files with
different values keep theirs. The preview takes its blur and its enabled
regions from the shown file's settings.

- **Preset row:** the preset the shown values started from, "Modified"
  when they differ, and a menu to reset, update a user preset, save as a
  new preset, import, export and delete. Built-ins (_Default_, _Max privacy_, _Kill feed
  only_) mirror `Preset::builtins`. Preset files of older revisions load;
  fields the engine never read (kept names, chat options, more
  verification options) are dropped.
- **Blur:** "All regions" and one switch per region in the HUD and
  Screens groups (world-space name tags are out of scope and say so), the
  manual boxes of the shown file, the style, and the box fit (Tight or
  Safe) with Re-analyze. An export analyses again when Safe is chosen for
  a file analysed with Tight boxes.
- **Export:** codec (disabled with the reason when this PC has no
  working encoder for it, read from `get_capabilities`), encoder, rate
  control, preset, 10-bit, NVENC sessions, audio, container, metadata,
  output folder, file name template and conflict policy; the output name
  and the size and time estimate come from `get_output_preview`.
- **Verify:** OCR on or off, the coverage (HUD activity or every
  frame), and the last result of the shown file. The report dialog shows stat tiles, the positive control per
  region, the leaks, the items OCR could not read and the names outside
  every region. Leak crops are the backend's blurred crops; "Reveal" asks
  the backend for the clear crop of that one leak and the image is
  dropped when the row closes. "Accept and rename" (outputs named
  `_UNVERIFIED`) calls `accept_report`.
- **Footer:** Analyze, Export, Export again or Cancel for the shown file,
  "Export N files" when several are selected, and the export summary
  (Ctrl+E runs the same action from anywhere in the workspace). Job
  events update the queue status (`src/features/jobs/jobs.ts`).

The request builders (`src/lib/job-request.ts`) put every inspector
value into the request unchanged; `inspector.test.tsx` changes every
control and compares the arguments the mock backend received with the
expected settings. The mock backend runs analysis and render jobs over
the job channel (`window.blurrdMock.jobs`: `durationMs`, `failNext`),
names outputs from the template (`_UNVERIFIED` until the project is
verified clean), keeps user presets in session storage and preset files
in `window.blurrdMock.presets.files`, and serves leak crops at
`blurrd://crop/<project>/<leak>.jpg?view=blurred|clear`.

Manual checks in the real window (not automated: no visible windows in
agent runs): the native folder dialog of "Choose folder", the open
dialog of "Import presets…" and the save dialog of "Export preset".

### Processing

An export of one file first verifies the frame ranges that edits changed
(`start_reverify`), then renders (`start_render`). "Export N files"
hands the selected files to the backend's run queue (`start_run`, one
job per file, run one after another); the files wait as "Waiting to
export" until their job starts. Events of a job whose id the command
has not answered yet are kept and delivered once it has. Operations that
overlap form a batch (`src/lib/batch.ts`):

- **Processing card** (`src/features/jobs/ProcessingCard.tsx`), above the
  transport while the shown file runs: the stages of its jobs, the
  overall progress, speed, frames and time left, Pause/Resume
  (`pause_queue`, `resume_queue`) and Cancel. Cancel works at every
  stage, for waiting files, and before the backend has answered the job
  id (it is sent with the id).
- **Queue progress** above the GPU card and the **taskbar button**
  (`setProgressBar`) show the batch's percentage: paused while the queue
  is paused, red once a file failed. The button only hears about changes
  of the whole percentage or the state.
- When the batch ends: one Windows notification (Settings › General,
  only while the window is in the background), and the folder of the
  last export opens if the user asked for that. Toasts name each result;
  "Show in folder" reveals the file, "Details" on a failure opens the
  message and the technical detail with "Copy details" (also in the queue
  menu: "Show error details…"). The empty state lists this session's
  exports (in memory only).

The frontend reacts to job events only, never to timers or animation
frames, so a minimised window does not slow the queue down; queue rows
follow progress at most four times a second, the card at the event
rate. The mock backend (`window.blurrdMock`) records the taskbar states
(`taskbar`), notifications (`notifications`, focus in `focused`) and
revealed files (`revealed`); its jobs run on their own clock, report at
20 Hz and support `paused`; `runAnswerDelayMs` lets `start_run` answer
after the first events.

Manual checks in the real window: the taskbar progress (normal, paused,
error), a notification while blurrd is minimised or behind another
window, and "Show in folder" opening File Explorer.

### Design tokens and typography

`src/styles/globals.css` defines every colour, text size and easing of the
user interface as a Tailwind theme token, with the light values in `@theme`
and the dark overrides under the `.dark` class (set on `<html>` by
`src/lib/appearance.ts` from Settings > Appearance, following the Windows
theme in "System" mode). Tailwind's default palette and text
scale are switched off: use the semantic tokens (`bg-panel`,
`text-foreground-subtle`, `border-border`, ...) and the `text-ui-*` sizes,
which all follow one interface-size variable. Compose class names with
`cn` from `src/lib/utils.ts`, which knows the `text-ui-*` sizes. Lucide
icons get their 1.5 px stroke from a provider at the root.

Reusable components live in `src/components/ui` (buttons, fields,
switches, selects, menus, tabs, dialogs, tooltips, toasts, ...), built on
the headless Radix UI primitives, and blurrd's own icons in
`src/components/icons` (24 px grid, 1.5 stroke, like Lucide). Keyboard
focus shows a 2 px accent ring on every control.

Each region has a colour family (`--color-region-*`): one per HUD region
and for manual boxes, and one shared by the four screen surfaces.
`RegionChip` always shows the region's label next to its colour.
`src/styles/contrast.test.ts` checks that primary and secondary text reach
4.5:1 on every surface (including hovered and selected rows), chip text
4.5:1 and every region colour 3:1 in both themes, so change the tokens only
together with that test passing.

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

| Job                     | Runner  | Checks                                                                                                                                                                                                                                                                 |
| ----------------------- | ------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `lint`                  | Ubuntu  | rustfmt, bindings and schema freshness, oxfmt, oxlint (warnings are errors), typography check, typos, gitleaks over the full history                                                                                                                                   |
| `frontend`              | Ubuntu  | `pnpm typecheck` (TypeScript 7), Vitest, `vite build`, blur reference frames (`cargo run -p blurrd-blur --example reference_frames`), Playwright end-to-end tests in Chromium against the mock backend with FFmpeg-encoded fixture videos (report uploaded on failure) |
| `screenshots (windows)` | Windows | Playwright screenshot comparison of the component gallery, the window shell and the floating layers in both themes (report uploaded on failure)                                                                                                                        |
| `rust (windows)`        | Windows | pinned FFmpeg (`cargo xtask fetch-ffmpeg`, cached), fixture fonts (`cargo xtask fetch-fonts`, cached), ONNX Runtime and OCR models (`cargo xtask fetch-ort`, `cargo xtask fetch-models`, cached), clippy with `-D warnings`, cargo-nextest, doctests                   |
| `deny`                  | Ubuntu  | `cargo deny --locked check advisories bans licenses sources` (`deny.toml`)                                                                                                                                                                                             |
| `bundle (windows)`      | Windows | pushes to `main` and manual runs only, after `frontend`, `screenshots (windows)` and `rust (windows)`: third-party notices freshness (`pnpm notices:check`), `pnpm build` (release app + NSIS installer), uploaded as the `blurrd-windows-<sha>` artifact for 14 days  |

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
- cargo-nextest, cargo-deny, cargo-about and gitleaks are installed by exact version from
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

## Third-party notices

`THIRD-PARTY-NOTICES.md` and the data of the app's open-source licences
view (`apps/desktop/src/features/about/licenses.json`) list everything the
installer and the CLI archive ship, with the full licence texts. Both are
generated by `scripts/licenses.mjs`; never edit them by hand.

```sh
pnpm notices         # regenerate both files
pnpm notices:check   # fail when either file is stale (CI bundle job)
```

The script merges, in this order:

1. The bundled programs and data: the FFmpeg build and the libraries in its
   DLLs (licence files and versions from `third_party/ffmpeg`, so run
   `cargo xtask fetch-ffmpeg` first; a library without a reviewed licence in
   the script's `FFMPEG_PORTS` stops it), ONNX Runtime and DirectML
   (`third_party/ort.toml` and the licence files `cargo xtask fetch-ort`
   copies) and the bundled OCR models (`models.toml`), once those pins exist.
2. The Rust crates of `blurrd-app` and `blurrd-cli` from cargo-about
   (`about.toml`: Windows x64 target, no build or dev dependencies, no
   workspace crates; `--fail` when a licence cannot be identified). The
   accepted licences mirror `deny.toml`.
3. The npm packages Vite bundles into the frontend: a production build into
   a temporary folder with Vite's `build.license` output, plus the packages
   `globals.css` imports (Tailwind CSS, tw-animate-css), whose CSS Vite does
   not attribute. Installed packages that are tree-shaken away are not
   listed. A package without a licence file must declare MIT and an author;
   it gets the standard MIT text.

Licence texts are deduplicated; the output is deterministic, so a stale
file shows up as a diff. Regenerate after every dependency change (the
bundle job fails otherwise) and before each release.

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
