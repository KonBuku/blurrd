//! Deciding whether a read of a blurred crop still shows a name.
//!
//! The verification reads the blurred pixels of every place where the
//! source showed a name and the blur plan does not cover it completely
//! (architecture plan 7.2, step 3). [`LeakMatcher`] classifies such a read:
//!
//! - **dictionary**: the read matches a name of the [`NameDictionary`]
//!   (a long enough common subsequence, or a run of four contiguous
//!   characters of a name of five or more; three matched HUD fragments on
//!   real recordings, a misread channel prefix for example);
//! - **dictionary-free**: the read is confident text of name length with
//!   letters that is neither a listed HUD word nor holds one (a label cut
//!   by a box), whether or not the name was ever read on the source (a
//!   name only a missed detector would have shown);
//! - otherwise nothing: the blur made the text unreadable.
//!
//! The matcher works on folded text ([`fold`]) and never keeps it.

use crate::dictionary::{NameDictionary, Whitelist, fold, name_len};

/// How a read was found to show a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NameMatch {
    /// It matches a name of the dictionary.
    Dictionary,
    /// It is confident text of name length outside the dictionary.
    DictionaryFree,
}

/// Classifies reads of blurred crops against the name dictionary.
#[derive(Debug, Clone, Copy)]
pub struct LeakMatcher<'a> {
    dictionary: &'a NameDictionary,
    whitelist: &'a Whitelist,
}

impl<'a> LeakMatcher<'a> {
    /// Reads below this confidence never count: blurred blocks make OCR
    /// produce low-confidence characters.
    pub const MIN_CONF: f32 = 0.3;
    /// A read outside the dictionary counts from this confidence (plan
    /// 7.2: forced recognition with confidence 0.5 in a name slot).
    pub const FREE_CONF: f32 = 0.5;
    /// A read outside the dictionary needs at least this many letters and
    /// digits; one or two characters identify nobody.
    pub const FREE_MIN_LEN: usize = 3;
    /// A read outside the dictionary that holds a listed word this long is
    /// a HUD label cut by a box, not a name.
    pub const LABEL_MIN_LEN: usize = 5;
    /// Shortest common subsequence that matches a name: `max(4, 0.7 ·
    /// name length)` characters.
    pub const SUBSEQUENCE_MIN: usize = 4;
    /// Fraction of a name's length a common subsequence must reach.
    pub const SUBSEQUENCE_SHARE: f32 = 0.7;
    /// Contiguous characters of a name that match it ...
    pub const RUN_MIN: usize = 4;
    /// ... when the name has at least this many characters.
    pub const RUN_NAME_MIN: usize = 5;

    /// A matcher for `dictionary`; reads that consist only of listed words
    /// never match.
    pub fn new(dictionary: &'a NameDictionary, whitelist: &'a Whitelist) -> Self {
        Self {
            dictionary,
            whitelist,
        }
    }

    /// How the read `text` with confidence `conf` (both from OCR of a
    /// blurred name slot) shows a name, or `None` when it does not.
    pub fn classify(&self, text: &str, conf: f32) -> Option<NameMatch> {
        if conf < Self::MIN_CONF {
            return None;
        }
        let folded = fold(text);
        if folded.is_empty() || self.whitelist.covers(text) || self.whitelist.contains(&folded) {
            return None;
        }
        if self.in_dictionary(&folded) {
            return Some(NameMatch::Dictionary);
        }
        // Numbers alone (scores, money, distances) identify nobody.
        let lettered = folded.chars().any(char::is_alphabetic);
        let label = self.whitelist.holds_word(&folded, Self::LABEL_MIN_LEN);
        (conf >= Self::FREE_CONF && lettered && !label && name_len(text) >= Self::FREE_MIN_LEN)
            .then_some(NameMatch::DictionaryFree)
    }

    /// Whether the folded read matches a name of the dictionary.
    fn in_dictionary(&self, folded: &str) -> bool {
        let read: Vec<char> = folded.chars().collect();
        self.dictionary.iter().any(|(name, _)| {
            let name: Vec<char> = name.chars().collect();
            matches_name(&read, &name)
        })
    }
}

/// Whether a folded read matches a folded name: a common subsequence of
/// at least `max(4, 0.7 · len)` characters, the whole name for names
/// shorter than that, or a run of [`LeakMatcher::RUN_MIN`] contiguous
/// characters of a name of five or more.
fn matches_name(read: &[char], name: &[char]) -> bool {
    let needed = LeakMatcher::SUBSEQUENCE_MIN
        .max((LeakMatcher::SUBSEQUENCE_SHARE * name.len() as f32).ceil() as usize);
    if name.len() < needed {
        return read == name;
    }
    common_subsequence(read, name) >= needed
        || (name.len() >= LeakMatcher::RUN_NAME_MIN
            && common_run(read, name) >= LeakMatcher::RUN_MIN)
}

/// Length of the longest common subsequence.
fn common_subsequence(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    for &ca in a {
        let mut diagonal = 0;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == cb {
                diagonal + 1
            } else {
                above.max(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// Length of the longest common run of contiguous characters.
fn common_run(a: &[char], b: &[char]) -> usize {
    let mut row = vec![0usize; b.len() + 1];
    let mut best = 0;
    for &ca in a {
        for j in (0..b.len()).rev() {
            row[j + 1] = if ca == b[j] { row[j] + 1 } else { 0 };
            best = best.max(row[j + 1]);
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::{LeakMatcher, NameMatch, common_run, common_subsequence};
    use crate::dictionary::{NameDictionary, Whitelist};

    fn chars(s: &str) -> Vec<char> {
        s.chars().collect()
    }

    #[test]
    fn measures_common_subsequences_and_runs() {
        assert_eq!(common_subsequence(&chars("novaprime"), &chars("nvprme")), 6);
        assert_eq!(common_subsequence(&chars("abc"), &chars("xyz")), 0);
        assert_eq!(common_run(&chars("xxnovay"), &chars("supernova")), 4);
        assert_eq!(common_run(&chars("xy"), &chars("yx")), 1);
    }

    #[test]
    fn classifies_reads_of_blurred_slots() {
        let mut dictionary = NameDictionary::new();
        dictionary.insert("Nova Prime", 0.95);
        dictionary.insert("Bo", 0.95);
        dictionary.insert("Kestrel", 0.95);
        let whitelist = Whitelist::hud_default();
        let matcher = LeakMatcher::new(&dictionary, &whitelist);
        // Whole or partly readable names.
        assert_eq!(
            matcher.classify("Nova Prime", 0.9),
            Some(NameMatch::Dictionary)
        );
        assert_eq!(
            matcher.classify("NvaPrim", 0.6),
            Some(NameMatch::Dictionary)
        );
        assert_eq!(matcher.classify("xstre", 0.35), Some(NameMatch::Dictionary));
        // Short names only as a whole.
        assert_eq!(matcher.classify("bo", 0.9), Some(NameMatch::Dictionary));
        assert_eq!(matcher.classify("bob", 0.4), None);
        // Confident text of name length outside the dictionary.
        assert_eq!(
            matcher.classify("Umbra", 0.8),
            Some(NameMatch::DictionaryFree)
        );
        assert_eq!(matcher.classify("Umbra", 0.45), None);
        assert_eq!(matcher.classify("Um", 0.9), None);
        assert_eq!(matcher.classify("4500", 0.99), None);
        assert_eq!(matcher.classify("oon.REPORT", 0.9), None);
        assert_eq!(matcher.classify("< > SWITCH PLAYER", 0.9), None);
        // Blur noise, HUD words and unconfident reads.
        assert_eq!(matcher.classify("Nova Prime", 0.2), None);
        assert_eq!(matcher.classify("TEAM ACE", 0.99), None);
        assert_eq!(matcher.classify("Raze", 0.99), None);
        assert_eq!(matcher.classify("--", 0.99), None);
    }
}
