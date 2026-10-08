# Contributing to blurrd

Thank you for your interest in blurrd. Bug reports, documentation and code
are welcome. blurrd is in early development, so please open an issue to
discuss larger changes before you start working on them.

## Ground rules

- Be kind and constructive. Everyone taking part in this project follows the
  [Code of Conduct](CODE_OF_CONDUCT.md).
- Report security problems privately as described in
  [SECURITY.md](SECURITY.md), never in public issues.
- **Never share other players' names.** Do not attach recordings, frames or
  screenshots in which names are readable, and do not paste OCR output that
  contains names. Describe a missed name with the timestamp, the HUD region
  and the resolution instead, or attach a still in which the name has been
  covered by hand.

## Media policy

blurrd is tested against real gameplay recordings, but those recordings
contain other players' names (personal data) and third-party copyrighted
material. They never enter the repository, not even temporarily:

- No clips, excerpts, frames, crops, OCR dictionaries, text hashes or golden
  files made from real footage. `.gitignore` blocks the common media and
  model formats; do not force-add them.
- Committed test fixtures are synthetic: HUD-like layouts with made-up names,
  rendered with an OFL-licensed font at test time.
- Tests that need real footage read the folder named by the
  `BLURRD_TEST_MEDIA` environment variable and are skipped when it is not
  set. Use your own recordings. `tests/media/manifest.toml` identifies the
  maintainers' recordings by checksum only.

## Development setup

You will need Windows 11 x64 with the MSVC build tools, Rust via
[rustup](https://rustup.rs) (the toolchain is pinned in
`rust-toolchain.toml`), LLVM 21 or newer and Node.js 24 LTS with pnpm 12.
[docs/development.md](docs/development.md) lists the exact tools and
explains how to build, test and check the code.

## Commit messages

blurrd follows [Conventional Commits 1.0.0](https://www.conventionalcommits.org/en/v1.0.0/):

```
<type>(<scope>): <description>

<body: what changed and why, wrapped at 72 characters>

<footers, for example "Refs #12" or "BREAKING CHANGE: ...">
```

- **Header:** at most 72 characters, imperative mood ("add", not "added"),
  lowercase, no trailing period.
- **Types:** `feat`, `fix`, `perf`, `refactor`, `docs`, `test`, `build`,
  `ci`, `chore`, `style` and `revert`.
- **`feat` or `fix`:** a `fix` repairs a bug in a released version (or in
  infrastructure that is already merged, such as CI). Work on a feature that
  has not been released yet is `feat`, `refactor` or `test`, never `fix`.
- **`perf`:** the body states before and after numbers and the machine they
  were measured on.
- **Scopes:** one of the following, or none when a change spans several
  areas.

  | Scope     | Area                                                      |
  | --------- | --------------------------------------------------------- |
  | `core`    | shared types, project schema, geometry, settings, errors  |
  | `blur`    | blur implementation                                       |
  | `video`   | probing, decoding, encoding, muxing, Windows system calls |
  | `detect`  | HUD and screen detectors, temporal tracking               |
  | `ocr`     | OCR runtime, models, leak matching                        |
  | `engine`  | analysis, verification, render and audit jobs             |
  | `cli`     | the `blurrd` command-line tool                            |
  | `app`     | the Tauri shell (`apps/desktop/src-tauri`) and packaging  |
  | `ui`      | the frontend (`apps/desktop/src`)                         |
  | `testkit` | synthetic fixtures, golden harness, metrics               |
  | `xtask`   | developer tasks (`cargo xtask ...`)                       |
  | `docs`    | documentation tooling                                     |
  | `ci`      | CI helpers                                                |
  | `deps`    | dependency updates                                        |
  | `release` | release commits                                           |
  | `repo`    | repository metadata and configuration                     |

- Keep one logical change per commit. Every commit builds and passes the
  checks on its own, and a dependency is added in the commit that first
  uses it.
- Do not add tool-generated trailers or "Generated with" lines.
- The `commit-msg` hook that `pnpm install` sets up checks these rules
  with commitlint before the commit is created.

## Pull requests

- Branch from `main` and name the branch `<type>/<topic>`, for example
  `feat/scoreboard-rows`.
- Keep a pull request to one topic. Its title is the Conventional Commits
  header of the squashed commit.
- Add tests for every change in behaviour, and update the documentation in
  the same pull request.
- Do not edit `CHANGELOG.md`; it is generated from the commit history.

## Licensing

blurrd is licensed under the [GNU General Public License 3.0 only](LICENSE).
By contributing, you agree that your contributions are licensed under the
same license (GPL-3.0-only). There is no contributor license agreement and no
sign-off requirement.
