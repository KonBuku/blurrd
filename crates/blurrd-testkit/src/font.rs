//! Fonts and text rendering for the synthetic HUD.
//!
//! Latin text uses Barlow Condensed in its semibold weight (SIL Open Font
//! License 1.1), committed in `tests/fonts/` and embedded at compile time.
//! It is a condensed sans in the spirit of the game's HUD font, not that
//! font: no game font or art is used anywhere in the fixtures.
//!
//! Names in other scripts use Noto Sans families (also OFL 1.1), which are
//! too large to commit. `cargo xtask fetch-fonts` downloads them, pinned by
//! SHA-256 in `tests/fonts/fonts.toml`, into `tests/fonts/noto/`
//! (gitignored). [`FontSet::with_fetched`] loads them; tests that need
//! them skip with a message when they are missing, unless
//! [`fetched_fonts_required`] says that they must be present (CI).
//!
//! Text is laid out glyph by glyph with kerning but without shaping, so
//! Arabic letters appear in their isolated forms and Devanagari conjuncts
//! are not formed. That is enough for what the fixtures test: ink of a
//! non-Latin script inside a name slot.

use std::fmt;
use std::path::{Path, PathBuf};

use ab_glyph::{Font, FontArc, FontVec, Glyph, GlyphId, PxScale, ScaleFont, VariableFont, point};

/// The embedded Latin font (Barlow Condensed, semibold, OFL 1.1).
const LATIN_FONT: &[u8] = include_bytes!("../../../tests/fonts/BarlowCondensed-SemiBold.ttf");

/// Weight the variable Noto fonts are set to, close to the semibold Latin
/// font.
const NOTO_WEIGHT: f32 = 600.0;

/// Environment variable that makes missing fetched fonts an error instead
/// of a skip; set in CI after `cargo xtask fetch-fonts`.
pub const REQUIRE_FONTS_ENV: &str = "BLURRD_REQUIRE_FONTS";

/// A writing system the generator can render names in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Script {
    /// Latin, including Vietnamese diacritics (embedded font).
    Latin,
    /// Korean Hangul (Noto Sans KR).
    Hangul,
    /// Japanese kana and kanji, which also covers Chinese Han characters
    /// (Noto Sans JP).
    Japanese,
    /// Thai (Noto Sans Thai).
    Thai,
    /// Arabic (Noto Sans Arabic), rendered without shaping.
    Arabic,
    /// Devanagari (Noto Sans Devanagari), rendered without shaping.
    Devanagari,
}

impl Script {
    /// The scripts that need fetched fonts.
    pub const FETCHED: [Self; 5] = [
        Self::Hangul,
        Self::Japanese,
        Self::Thai,
        Self::Arabic,
        Self::Devanagari,
    ];

    /// File name of the script's font in `tests/fonts/noto/` (the name
    /// `cargo xtask fetch-fonts` writes), or `None` for the embedded font.
    pub const fn file_name(self) -> Option<&'static str> {
        match self {
            Self::Latin => None,
            Self::Hangul => Some("NotoSansKR.ttf"),
            Self::Japanese => Some("NotoSansJP.ttf"),
            Self::Thai => Some("NotoSansThai.ttf"),
            Self::Arabic => Some("NotoSansArabic.ttf"),
            Self::Devanagari => Some("NotoSansDevanagari.ttf"),
        }
    }
}

/// Why the fetched fonts could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingFonts {
    /// The folder that was searched.
    pub dir: PathBuf,
    /// The font files that are missing or unreadable.
    pub files: Vec<String>,
}

impl fmt::Display for MissingFonts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "fonts {} not found in {} (run `cargo xtask fetch-fonts`)",
            self.files.join(", "),
            self.dir.display()
        )
    }
}

impl std::error::Error for MissingFonts {}

/// Whether tests must fail instead of skipping when the fetched fonts are
/// missing (the variable [`REQUIRE_FONTS_ENV`] is set to a non-empty value
/// other than `0`).
pub fn fetched_fonts_required() -> bool {
    std::env::var(REQUIRE_FONTS_ENV).is_ok_and(|v| !v.is_empty() && v != "0")
}

/// The default folder of the fetched fonts, `tests/fonts/noto/` in the
/// repository.
pub fn fetched_fonts_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("fonts")
        .join("noto")
}

/// Size of rendered text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextStyle {
    /// Height of a capital letter (`H`) in pixels.
    pub cap_px: f32,
    /// Horizontal stretch (1.0 = the font's own proportions); used for
    /// stretched HUD layouts.
    pub x_scale: f32,
}

impl TextStyle {
    /// Text whose capitals are `cap_px` pixels tall, unstretched.
    pub const fn cap(cap_px: f32) -> Self {
        Self {
            cap_px,
            x_scale: 1.0,
        }
    }
}

/// A rendered line of text: anti-aliased coverage in `0.0..=1.0`.
#[derive(Debug, Clone, PartialEq)]
pub struct TextBitmap {
    /// Width of the coverage map in pixels.
    pub width: u32,
    /// Height of the coverage map in pixels.
    pub height: u32,
    /// Row of the baseline, counted from the top of the map (may lie
    /// outside the map for text without glyphs below or above it).
    pub baseline: i32,
    /// Column of the pen start (the text's origin), counted from the left
    /// of the map; negative side bearings put ink left of it.
    pub origin_x: i32,
    /// Distance the pen moved, in pixels.
    pub advance: f32,
    /// Coverage per pixel, row after row.
    pub coverage: Vec<f32>,
}

impl TextBitmap {
    /// Coverage at `(x, y)` of the map, 0 outside.
    pub fn at(&self, x: i32, y: i32) -> f32 {
        if x < 0 || y < 0 || x >= self.width as i32 || y >= self.height as i32 {
            0.0
        } else {
            self.coverage[y as usize * self.width as usize + x as usize]
        }
    }
}

/// The fonts available to the generator.
#[derive(Debug, Clone)]
pub struct FontSet {
    /// Latin first, then the fetched fonts in [`Script::FETCHED`] order.
    fonts: Vec<(Script, FontArc)>,
}

impl FontSet {
    /// The embedded Latin font only. Enough for every fixture except names
    /// in other scripts.
    pub fn latin() -> Self {
        // The embedded font is a fixed, valid TrueType file; parsing it can
        // only fail if the repository is corrupt, which every test using
        // fonts would then show as missing glyphs.
        let fonts = FontArc::try_from_slice(LATIN_FONT)
            .map(|font| vec![(Script::Latin, font)])
            .unwrap_or_default();
        Self { fonts }
    }

    /// The Latin font plus every fetched font from [`fetched_fonts_dir`].
    ///
    /// # Errors
    ///
    /// [`MissingFonts`] lists the fonts that are missing or unreadable.
    pub fn with_fetched() -> Result<Self, MissingFonts> {
        Self::with_fetched_from(&fetched_fonts_dir())
    }

    /// The Latin font plus every fetched font from `dir`.
    ///
    /// # Errors
    ///
    /// [`MissingFonts`] lists the fonts that are missing or unreadable.
    pub fn with_fetched_from(dir: &Path) -> Result<Self, MissingFonts> {
        let mut set = Self::latin();
        let mut missing = Vec::new();
        for script in Script::FETCHED {
            let Some(name) = script.file_name() else {
                continue;
            };
            let font = std::fs::read(dir.join(name))
                .ok()
                .and_then(|bytes| FontVec::try_from_vec(bytes).ok())
                .map(|mut font| {
                    // The fetched files are variable fonts whose default
                    // instance may be a light weight; fonts without the
                    // axis keep their own weight.
                    font.set_variation(b"wght", NOTO_WEIGHT);
                    FontArc::new(font)
                });
            match font {
                Some(font) => set.fonts.push((script, font)),
                None => missing.push(name.to_owned()),
            }
        }
        if missing.is_empty() {
            Ok(set)
        } else {
            Err(MissingFonts {
                dir: dir.to_path_buf(),
                files: missing,
            })
        }
    }

    /// Whether a font for `script` is loaded.
    pub fn has(&self, script: Script) -> bool {
        self.fonts.iter().any(|(s, _)| *s == script)
    }

    /// The first loaded font that has a glyph for `c`, preferring Latin.
    fn font_for(&self, c: char) -> Option<&FontArc> {
        self.fonts
            .iter()
            .map(|(_, f)| f)
            .find(|f| f.glyph_id(c) != GlyphId(0))
            .or_else(|| self.fonts.first().map(|(_, f)| f))
    }

    /// Whether every character of `text` (except spaces) has a glyph in
    /// one of the loaded fonts.
    pub fn covers(&self, text: &str) -> bool {
        text.chars()
            .filter(|c| !c.is_whitespace())
            .all(|c| self.fonts.iter().any(|(_, f)| f.glyph_id(c) != GlyphId(0)))
    }

    /// Renders `text` on one line.
    pub fn render(&self, text: &str, style: TextStyle) -> TextBitmap {
        struct Placed {
            outline: ab_glyph::OutlinedGlyph,
        }
        let mut pen = 0.0f32;
        let mut previous: Option<(&FontArc, GlyphId)> = None;
        let mut placed = Vec::new();
        for c in text.chars() {
            let Some(font) = self.font_for(c) else {
                continue;
            };
            let scale = px_scale(font, style);
            let scaled = font.as_scaled(scale);
            let id = font.glyph_id(c);
            if let Some((prev_font, prev_id)) = previous
                && std::ptr::eq(prev_font, font)
            {
                pen += scaled.kern(prev_id, id);
            }
            let glyph = Glyph {
                id,
                scale,
                position: point(pen, 0.0),
            };
            if let Some(outline) = font.outline_glyph(glyph) {
                placed.push(Placed { outline });
            }
            pen += scaled.h_advance(id);
            previous = Some((font, id));
        }
        let (mut x0, mut y0, mut x1, mut y1) = (0i32, 0i32, pen.ceil() as i32, 1i32);
        for p in &placed {
            let b = p.outline.px_bounds();
            x0 = x0.min(b.min.x as i32);
            y0 = y0.min(b.min.y as i32);
            x1 = x1.max(b.max.x as i32);
            y1 = y1.max(b.max.y as i32);
        }
        let (width, height) = ((x1 - x0).max(1) as u32, (y1 - y0).max(1) as u32);
        let mut coverage = vec![0.0f32; width as usize * height as usize];
        for p in &placed {
            let b = p.outline.px_bounds();
            let (ox, oy) = (b.min.x as i32 - x0, b.min.y as i32 - y0);
            p.outline.draw(|gx, gy, c| {
                let (x, y) = (ox + gx as i32, oy + gy as i32);
                if x >= 0 && y >= 0 && (x as u32) < width && (y as u32) < height {
                    let cell = &mut coverage[y as usize * width as usize + x as usize];
                    *cell = cell.max(c.clamp(0.0, 1.0));
                }
            });
        }
        TextBitmap {
            width,
            height,
            baseline: -y0,
            origin_x: -x0,
            advance: pen,
            coverage,
        }
    }

    /// Width of `text` in pixels (the pen advance).
    pub fn measure(&self, text: &str, style: TextStyle) -> f32 {
        self.render(text, style).advance
    }
}

/// The pixel scale at which `font`'s capital `H` is `style.cap_px` tall.
fn px_scale(font: &FontArc, style: TextStyle) -> PxScale {
    let height = font.height_unscaled();
    let cap_units = font
        .outline(font.glyph_id('H'))
        .map(|o| o.bounds.height())
        .filter(|h| *h > 0.0)
        .or_else(|| font.units_per_em().map(|em| em * 0.7))
        .unwrap_or(height * 0.6);
    let y = style.cap_px * height / cap_units;
    PxScale {
        x: y * style.x_scale,
        y,
    }
}

#[cfg(test)]
mod tests {
    use super::{FontSet, MissingFonts, Script, TextStyle};

    #[test]
    fn renders_latin_with_the_requested_cap_height() {
        let fonts = FontSet::latin();
        assert!(fonts.has(Script::Latin));
        let bmp = fonts.render("H", TextStyle::cap(14.0));
        let rows: Vec<i32> = (0..bmp.height as i32)
            .filter(|&y| (0..bmp.width as i32).any(|x| bmp.at(x, y) > 0.5))
            .collect();
        let ink_height = rows.last().unwrap() - rows.first().unwrap() + 1;
        assert!((13..=15).contains(&ink_height), "cap height {ink_height}");
        assert!(bmp.baseline > 0 && bmp.baseline <= bmp.height as i32);
    }

    #[test]
    fn stretches_horizontally() {
        let fonts = FontSet::latin();
        let narrow = fonts.measure("Wide name", TextStyle::cap(14.0));
        let wide = fonts.measure(
            "Wide name",
            TextStyle {
                cap_px: 14.0,
                x_scale: 1.5,
            },
        );
        assert!((wide / narrow - 1.5).abs() < 0.01);
    }

    #[test]
    fn covers_vietnamese_with_the_latin_font() {
        let fonts = FontSet::latin();
        assert!(fonts.covers("Trường Ngữ"));
        assert!(!fonts.covers("한글"));
    }

    #[test]
    fn reports_missing_fetched_fonts() {
        let dir = std::env::temp_dir().join("blurrd-testkit-no-fonts");
        let err: MissingFonts = FontSet::with_fetched_from(&dir).unwrap_err();
        assert_eq!(err.files.len(), Script::FETCHED.len());
        assert!(err.to_string().contains("cargo xtask fetch-fonts"));
    }
}
