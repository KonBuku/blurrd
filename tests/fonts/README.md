# Test fonts

Fonts used by `blurrd-testkit` to render the synthetic HUD fixtures. None
of them is the game's own font.

| File                           | Font                                       | Source                                                                                                            | License                                |
| ------------------------------ | ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------- | -------------------------------------- |
| `BarlowCondensed-SemiBold.ttf` | Barlow Condensed, semibold                 | [google/fonts](https://github.com/google/fonts/tree/5e8a3ba899557829a76cfdac30fa512bda91d7ca/ofl/barlowcondensed) | SIL OFL 1.1, `OFL.txt`                 |
| `noto/*` (not committed)       | Noto Sans KR, JP, Thai, Arabic, Devanagari | pinned in `fonts.toml`, fetched by `cargo xtask fetch-fonts`                                                      | SIL OFL 1.1, fetched next to each font |

The Barlow file is the unmodified upstream file (SHA-256
`7b619d14bc2327509a9ef32b0890f709626f7ecc9ff61191c2a4314c5499d2d9`).
