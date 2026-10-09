//! The name dictionary and the words that are never names.
//!
//! The dictionary holds the player names OCR read on the unblurred source.
//! It lives only in memory: it is never written to disk, logged or sent
//! anywhere, and reports contain no text or text-derived values
//! (architecture plan 7.2 and 10). Names are compared in a folded form
//! ([`fold`]): case-folded letters and digits of any script, Latin letters
//! without accents, no spaces or punctuation.

use std::collections::{BTreeSet, HashMap};

use unicode_normalization::char::decompose_canonical;

/// The folded form of `text` that names are stored and compared in:
/// letters and digits of any script, lowercased; Latin letters lose their
/// accents (`Hồng` and `Hong` are the same name, because OCR often misreads
/// diacritics, the same way on the source and on the blurred output);
/// full-width Latin is mapped to ASCII; spaces, punctuation and symbols are
/// dropped. Other scripts are kept as they are.
pub fn fold(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        let c = full_width_to_ascii(c);
        if !c.is_alphanumeric() {
            continue;
        }
        for lower in c.to_lowercase() {
            push_latin_base(lower, &mut out);
        }
    }
    out
}

/// Number of name characters in `text`: the letters and digits as written
/// (before folding, which may change the count).
pub fn name_len(text: &str) -> usize {
    text.chars()
        .map(full_width_to_ascii)
        .filter(|c| c.is_alphanumeric())
        .count()
}

/// Maps full-width ASCII forms (U+FF01..U+FF5E) to ASCII.
fn full_width_to_ascii(c: char) -> char {
    if ('\u{ff01}'..='\u{ff5e}').contains(&c) {
        char::from_u32(c as u32 - 0xfee0).unwrap_or(c)
    } else {
        c
    }
}

/// Appends `c` without accents if its canonical decomposition starts with
/// an ASCII letter; Latin letters without a decomposition that OCR reads
/// as plain letters get their usual base; anything else stays as it is.
fn push_latin_base(c: char, out: &mut String) {
    let base = match c {
        'đ' | 'ð' => Some("d"),
        'ø' => Some("o"),
        'ł' => Some("l"),
        'ı' => Some("i"),
        'ß' => Some("ss"),
        'æ' => Some("ae"),
        'œ' => Some("oe"),
        _ => None,
    };
    if let Some(base) = base {
        out.push_str(base);
        return;
    }
    let mut first = None;
    decompose_canonical(c, |d| {
        first.get_or_insert(d);
    });
    match first {
        Some(d) if d.is_ascii_alphabetic() => out.push(d),
        _ => out.push(c),
    }
}

/// Words that appear in the protected zones but are not player names: HUD
/// labels, banner titles, agent names, map callouts and location words.
/// They never enter the dictionary and never count as leaks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Whitelist {
    words: BTreeSet<String>,
}

impl Whitelist {
    /// HUD words, banner titles, system words and the location words of
    /// the voice-chat lines of the supported client languages.
    const HUD: &'static [&'static str] = &[
        "ace",
        "clutch",
        "teamace",
        "flawless",
        "thrifty",
        "team",
        "all",
        "party",
        "system",
        "broadcast",
        "won",
        "lost",
        "spike",
        "planted",
        "defused",
        "detonated",
        "ult",
        "ready",
        "ultready",
        "signature",
        "ability",
        "charged",
        "round",
        "match",
        "victory",
        "defeat",
        "buy",
        "phase",
        "attackers",
        "defenders",
        "attack",
        "defense",
        "kill",
        "killed",
        "by",
        "you",
        "headshot",
        "wallbang",
        "first",
        "blood",
        "spectating",
        "in",
        "en",
        "im",
        "el",
        "la",
        "alle",
        "todos",
        "equipo",
    ];

    /// Agent names (they appear in chat and on the recap screens).
    const AGENTS: &'static [&'static str] = &[
        "astra",
        "breach",
        "brimstone",
        "chamber",
        "clove",
        "cypher",
        "deadlock",
        "fade",
        "gekko",
        "harbor",
        "iso",
        "jett",
        "kayo",
        "killjoy",
        "neon",
        "omen",
        "phoenix",
        "raze",
        "reyna",
        "sage",
        "skye",
        "sova",
        "tejo",
        "viper",
        "vyse",
        "waylay",
        "yoru",
        "veto",
    ];

    /// Map callouts of the voice-chat location line.
    const CALLOUTS: &'static [&'static str] = &[
        "mid",
        "top",
        "bottom",
        "tiles",
        "market",
        "elbow",
        "nest",
        "pipes",
        "heaven",
        "hell",
        "main",
        "link",
        "courtyard",
        "garage",
        "tower",
        "window",
        "lobby",
        "site",
        "long",
        "short",
        "spawn",
        "ramp",
        "hookah",
        "showers",
        "lamps",
        "screens",
        "catwalk",
        "door",
        "doors",
        "connector",
        "boathouse",
        "rafters",
        "pit",
        "hall",
        "back",
        "front",
        "upper",
        "lower",
        "a",
        "b",
        "c",
    ];

    /// The default list: HUD words and banner titles, system and location
    /// words of the supported client languages, agent names and map
    /// callouts.
    pub fn hud_default() -> Self {
        Self::from_words(
            Self::HUD
                .iter()
                .chain(Self::AGENTS)
                .chain(Self::CALLOUTS)
                .copied(),
        )
    }

    /// A list of the given words (folded).
    pub fn from_words<'w>(words: impl IntoIterator<Item = &'w str>) -> Self {
        Self {
            words: words
                .into_iter()
                .map(fold)
                .filter(|w| !w.is_empty())
                .collect(),
        }
    }

    /// Whether the folded text is a listed word.
    pub fn contains(&self, folded: &str) -> bool {
        self.words.contains(folded)
    }

    /// Whether the folded text is a run of listed words without separators
    /// (OCR sometimes drops the spaces of a location line, giving
    /// `inasiteteam`).
    pub fn segments(&self, folded: &str) -> bool {
        let chars: Vec<(usize, char)> = folded.char_indices().collect();
        if chars.is_empty() {
            return false;
        }
        // reachable[i]: the first i characters split into listed words.
        let mut reachable = vec![false; chars.len() + 1];
        reachable[0] = true;
        for end in 1..=chars.len() {
            let end_byte = chars.get(end).map_or(folded.len(), |&(b, _)| b);
            reachable[end] = (0..end).any(|start| {
                reachable[start] && self.words.contains(&folded[chars[start].0..end_byte])
            });
        }
        reachable[chars.len()]
    }

    /// Whether every space-separated word of `text` is listed (a location
    /// line such as "in B Main" or a title such as "TEAM ACE").
    pub fn covers(&self, text: &str) -> bool {
        let mut any = false;
        for word in text.split_whitespace() {
            let folded = fold(word);
            if folded.is_empty() {
                continue;
            }
            if !self.contains(&folded) {
                return false;
            }
            any = true;
        }
        any
    }
}

/// How often and how well a name was read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NameStats {
    /// Reads of the name.
    pub count: u32,
    /// Best recognition confidence among them.
    pub best_conf: f32,
}

/// The player names read on the source, in folded form, in memory only.
#[derive(Debug, Clone, Default)]
pub struct NameDictionary {
    names: HashMap<String, NameStats>,
}

impl NameDictionary {
    /// Reads below this confidence are not trusted as names.
    pub const MIN_CONF: f32 = 0.8;
    /// Shortest name, in letters and digits.
    pub const MIN_LEN: usize = 2;
    /// Longest name, in letters and digits; game names have at most 16
    /// characters.
    pub const MAX_LEN: usize = 16;

    /// An empty dictionary.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a read of a name. Returns `false` (and adds nothing) when the
    /// confidence is below [`Self::MIN_CONF`] or the name has fewer than
    /// [`Self::MIN_LEN`] or more than [`Self::MAX_LEN`] letters and digits
    /// ([`name_len`]).
    pub fn insert(&mut self, raw: &str, conf: f32) -> bool {
        let len = name_len(raw);
        if conf < Self::MIN_CONF || !(Self::MIN_LEN..=Self::MAX_LEN).contains(&len) {
            return false;
        }
        let folded = fold(raw);
        let entry = self.names.entry(folded).or_insert(NameStats {
            count: 0,
            best_conf: 0.0,
        });
        entry.count += 1;
        entry.best_conf = entry.best_conf.max(conf);
        true
    }

    /// Number of distinct names.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    /// Whether no name was read yet.
    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }

    /// Whether `raw` (folded) is a known name.
    pub fn contains(&self, raw: &str) -> bool {
        self.names.contains_key(&fold(raw))
    }

    /// The folded names with their statistics, in no particular order.
    pub fn iter(&self) -> impl Iterator<Item = (&str, NameStats)> + '_ {
        self.names
            .iter()
            .map(|(name, stats)| (name.as_str(), *stats))
    }
}

#[cfg(test)]
mod tests {
    use super::{NameDictionary, Whitelist, fold, name_len};

    #[test]
    fn folds_case_accents_spaces_and_punctuation() {
        assert_eq!(fold("xX_Nova Prime_Xx"), "xxnovaprimexx");
        assert_eq!(fold("ĐỨC Anh"), "ducanh");
        assert_eq!(fold("Hồng Lảo"), fold("Hōng Lào"));
        assert_eq!(fold("Søren Straße"), "sorenstrasse");
        assert_eq!(fold("Ｎｏｖａ１"), "nova1");
        assert_eq!(fold("김민준"), "김민준");
        assert_eq!(fold("ひかり"), "ひかり");
        assert_eq!(fold("#EUW 42"), "euw42");
        assert_eq!(name_len("김민준 x"), 4);
    }

    #[test]
    fn dictionary_keeps_confident_names_of_plausible_length() {
        let mut dict = NameDictionary::new();
        assert!(dict.insert("Nova Prime", 0.95));
        assert!(dict.insert("nova prime", 0.85));
        assert!(!dict.insert("Nova Prime", 0.5));
        assert!(!dict.insert("x", 0.99));
        assert!(!dict.insert("abcdefghijklmnopq", 0.99));
        assert!(dict.insert("하나", 0.9));
        assert_eq!(dict.len(), 2);
        assert!(dict.contains("NOVA-PRIME"));
        let (_, stats) = dict.iter().find(|(n, _)| *n == "novaprime").unwrap();
        assert_eq!(stats.count, 2);
        assert!((stats.best_conf - 0.95).abs() < 1e-6);
    }

    #[test]
    fn whitelist_covers_hud_words_and_location_lines() {
        let list = Whitelist::hud_default();
        assert!(list.contains("ace"));
        assert!(list.covers("TEAM ACE"));
        assert!(list.covers("in B Main"));
        assert!(!list.covers("in B Main with Nova"));
        assert!(!list.covers("   "));
        assert!(list.segments("inasiteteam"));
        assert!(list.segments("inbmainteam"));
        assert!(!list.segments("inasiteteamnova"));
        assert!(!list.segments(""));
        assert!(Whitelist::from_words(["Foo Bar"]).contains("foobar"));
    }
}
