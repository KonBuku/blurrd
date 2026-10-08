//! Made-up player names.
//!
//! Names are generated from a seed out of random syllables or random
//! characters of a script, so no real player's name ever appears in a
//! fixture or a test.

use crate::font::Script;
use crate::rng::Rng;

/// The longest name the game allows, in characters.
pub const MAX_NAME_CHARS: usize = 16;

/// What kind of name to generate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NameStyle {
    /// Latin letters (mixed case, sometimes with digits) of exactly this
    /// many characters (clamped to `1..=16`).
    Latin(usize),
    /// Two Latin words separated by a space.
    TwoWord,
    /// Digits only.
    Digits(usize),
    /// Latin words with Vietnamese diacritics.
    Vietnamese,
    /// Characters of a non-Latin script (needs the fetched fonts).
    Script(Script),
}

impl NameStyle {
    /// The script the name is written in.
    pub const fn script(self) -> Script {
        match self {
            Self::Script(s) => s,
            _ => Script::Latin,
        }
    }

    /// A representative mix of styles for fixture matrices: the shortest
    /// and longest Latin names, typical lengths, two words, digits and
    /// Vietnamese.
    pub const LATIN_MIX: [Self; 7] = [
        Self::Latin(3),
        Self::Latin(7),
        Self::Latin(11),
        Self::Latin(MAX_NAME_CHARS),
        Self::TwoWord,
        Self::Digits(6),
        Self::Vietnamese,
    ];
}

const ONSETS: [&str; 18] = [
    "b", "d", "f", "g", "k", "l", "m", "n", "p", "r", "s", "t", "v", "z", "br", "kr", "st", "tr",
];
const VOWELS: [&str; 7] = ["a", "e", "i", "o", "u", "ai", "ou"];
const CODAS: [&str; 7] = ["", "", "n", "x", "r", "s", "th"];
const VIET_ONSETS: [&str; 8] = ["Ng", "Tr", "Ph", "Th", "H", "L", "V", "Đ"];
const VIET_RIMES: [&str; 12] = [
    "ường", "ữ", "ạnh", "ồng", "ắc", "iệt", "ảo", "ơn", "ầm", "ứa", "ỹ", "ọc",
];

/// Generates a name of the given style.
pub fn synth_name(rng: &mut Rng, style: NameStyle) -> String {
    match style {
        NameStyle::Latin(len) => latin(rng, len.clamp(1, MAX_NAME_CHARS)),
        NameStyle::TwoWord => {
            let first = rng.range(3, 7) as usize;
            let second = rng.range(3, (MAX_NAME_CHARS - 1 - first).min(7) as i32) as usize;
            format!("{} {}", latin(rng, first), latin(rng, second))
        }
        NameStyle::Digits(len) => (0..len.clamp(1, MAX_NAME_CHARS))
            .map(|_| char::from(b'0' + rng.below(10) as u8))
            .collect(),
        NameStyle::Vietnamese => {
            let mut words: Vec<String> = Vec::new();
            for _ in 0..2 {
                words.push(format!(
                    "{}{}",
                    rng.pick(&VIET_ONSETS),
                    rng.pick(&VIET_RIMES)
                ));
            }
            words.join(" ")
        }
        NameStyle::Script(script) => script_name(rng, script),
    }
}

/// Latin letters from syllables, cut or padded to exactly `len`
/// characters, with a capital first letter and sometimes trailing digits.
fn latin(rng: &mut Rng, len: usize) -> String {
    let mut s = String::new();
    while s.chars().count() < len {
        s.push_str(rng.pick(&ONSETS));
        s.push_str(rng.pick(&VOWELS));
        s.push_str(rng.pick(&CODAS));
    }
    let mut chars: Vec<char> = s.chars().take(len).collect();
    if len >= 5 && rng.chance(0.3) {
        let digits = rng.range(1, 2) as usize;
        for c in chars.iter_mut().rev().take(digits) {
            *c = char::from(b'0' + rng.below(10) as u8);
        }
    }
    if let Some(first) = chars.first_mut() {
        *first = first.to_ascii_uppercase();
    }
    if len >= 6 && rng.chance(0.3) {
        let i = rng.range(1, len as i32 - 1) as usize;
        chars[i] = chars[i].to_ascii_uppercase();
    }
    chars.into_iter().collect()
}

/// Random characters of a non-Latin script.
fn script_name(rng: &mut Rng, script: Script) -> String {
    let (ranges, count): (&[(u32, u32)], i32) = match script {
        Script::Latin => return latin(rng, 8),
        // Precomposed Hangul syllables.
        Script::Hangul => (&[(0xAC00, 0xD7A3)], rng.range(2, 5)),
        // Hiragana, katakana and common kanji.
        Script::Japanese => (
            &[(0x3041, 0x3093), (0x30A1, 0x30F3), (0x4E00, 0x4FFF)],
            rng.range(2, 6),
        ),
        // Thai consonants with a few vowels.
        Script::Thai => (&[(0x0E01, 0x0E2E), (0x0E32, 0x0E33)], rng.range(3, 8)),
        // Arabic letters.
        Script::Arabic => (&[(0x0627, 0x063A), (0x0641, 0x064A)], rng.range(3, 8)),
        // Devanagari consonants and vowel signs.
        Script::Devanagari => (&[(0x0915, 0x0939), (0x093E, 0x094C)], rng.range(3, 7)),
    };
    (0..count)
        .filter_map(|_| {
            let (lo, hi) = *rng.pick(ranges);
            char::from_u32(lo + rng.below(hi - lo + 1))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{MAX_NAME_CHARS, NameStyle, synth_name};
    use crate::font::Script;
    use crate::rng::Rng;

    #[test]
    fn latin_names_have_the_requested_length() {
        let mut rng = Rng::new(3);
        for len in [1, 3, 9, MAX_NAME_CHARS, 40] {
            let name = synth_name(&mut rng, NameStyle::Latin(len));
            assert_eq!(name.chars().count(), len.min(MAX_NAME_CHARS), "{name}");
            assert!(name.chars().all(|c| c.is_ascii_alphanumeric()));
        }
    }

    #[test]
    fn two_word_names_fit_the_limit() {
        let mut rng = Rng::new(5);
        for _ in 0..100 {
            let name = synth_name(&mut rng, NameStyle::TwoWord);
            assert_eq!(name.split(' ').count(), 2, "{name}");
            assert!(name.chars().count() <= MAX_NAME_CHARS);
        }
    }

    #[test]
    fn digit_and_script_names_use_their_alphabet() {
        let mut rng = Rng::new(8);
        assert!(
            synth_name(&mut rng, NameStyle::Digits(6))
                .chars()
                .all(|c| c.is_ascii_digit())
        );
        let hangul = synth_name(&mut rng, NameStyle::Script(Script::Hangul));
        assert!(
            hangul
                .chars()
                .all(|c| ('\u{AC00}'..='\u{D7A3}').contains(&c))
        );
        assert!(!synth_name(&mut rng, NameStyle::Vietnamese).is_ascii());
    }

    #[test]
    fn is_deterministic() {
        let a = synth_name(&mut Rng::new(11), NameStyle::Latin(12));
        let b = synth_name(&mut Rng::new(11), NameStyle::Latin(12));
        assert_eq!(a, b);
    }
}
