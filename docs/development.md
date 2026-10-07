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
| `xtask`              | Developer tasks, run with `cargo xtask <task>` (`cargo xtask help` lists them).      |
| `docs`               | Developer and user documentation.                                                    |
| `assets/brand`       | Logo and brand assets with their generator.                                          |

## Rust workspace

All commands run from the repository root.

```sh
cargo build --workspace                               # debug build
cargo nextest run --workspace                         # unit and integration tests
cargo test --doc --workspace                          # doctests (nextest does not run them)
cargo clippy --workspace --all-targets -- -D warnings # lints, as in CI
cargo fmt --all                                       # format Rust code
cargo deny check                                      # advisories, licenses, bans, sources
typos                                                 # spelling in code and docs
```

Lints are configured once in the root `Cargo.toml` (`[workspace.lints]`) and
inherited by every crate with `[lints] workspace = true`. Production code must
not use `unwrap`, `expect`, `dbg!`, `todo!` or `println!`; tests may unwrap,
expect and print (`clippy.toml`).

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
