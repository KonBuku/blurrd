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
| `scripts`            | Node scripts for repository tasks (`release.mjs`).                                   |
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

| Job              | Runner  | Checks                                                                              |
| ---------------- | ------- | ----------------------------------------------------------------------------------- |
| `lint`           | Ubuntu  | rustfmt, oxfmt, oxlint (warnings are errors), typos, gitleaks over the full history |
| `rust (windows)` | Windows | clippy with `-D warnings`, cargo-nextest, doctests with `RUSTDOCFLAGS=-D warnings`  |
| `deny`           | Ubuntu  | `cargo deny --locked check advisories bans licenses sources` (`deny.toml`)          |

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
