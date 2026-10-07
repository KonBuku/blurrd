//! Regions of the screen in which names are blurred, and the kinds of names.

use std::fmt;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// A screen region that can contain player names.
///
/// The detector regions are the gameplay HUD parts that blurrd finds by
/// itself. [`Region::Manual`] marks boxes the user drew; they are always
/// applied, whichever regions are enabled. World-space name tags above
/// players' heads are out of scope and have no region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum Region {
    /// Kill feed rows in the top right corner (killer and victim names).
    KillFeed,
    /// ACE, clutch and similar banners in the top centre.
    Banner,
    /// The voice-chat speaker list on the left.
    VoiceChat,
    /// Sender names in the text chat on the left.
    TextChat,
    /// A box drawn by the user.
    Manual,
}

impl Region {
    /// Every region, in declaration order.
    pub const ALL: [Self; 5] = [
        Self::KillFeed,
        Self::Banner,
        Self::VoiceChat,
        Self::TextChat,
        Self::Manual,
    ];

    /// The regions blurrd detects by itself ("all supported regions").
    pub const DETECTED: [Self; 4] = [
        Self::KillFeed,
        Self::Banner,
        Self::VoiceChat,
        Self::TextChat,
    ];

    /// Position of the region in [`Region::ALL`]; used as a bit index.
    const fn index(self) -> u32 {
        match self {
            Self::KillFeed => 0,
            Self::Banner => 1,
            Self::VoiceChat => 2,
            Self::TextChat => 3,
            Self::Manual => 4,
        }
    }

    /// A stable machine-readable key, as used on the command line.
    pub const fn key(self) -> &'static str {
        match self {
            Self::KillFeed => "killfeed",
            Self::Banner => "banner",
            Self::VoiceChat => "voice",
            Self::TextChat => "chat",
            Self::Manual => "manual",
        }
    }
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.key())
    }
}

/// What a blurred name is, inside its region.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[non_exhaustive]
pub enum NameKind {
    /// The killer in a kill feed row.
    Killer,
    /// The victim in a kill feed row.
    Victim,
    /// The player named in a banner.
    Banner,
    /// A speaker in the voice-chat list.
    Speaker,
    /// The sender of a text-chat line.
    ChatSender,
    /// A box drawn by the user.
    Manual,
    /// A slot box added by the OCR verification because a name was still
    /// readable.
    OcrFix,
}

/// A set of [`Region`]s.
///
/// Serialised as a sorted array of region names, for example
/// `["KillFeed", "Banner"]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize, TS)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(from = "Vec<Region>", into = "Vec<Region>")]
#[ts(as = "Vec<Region>")]
pub struct RegionSet {
    bits: u32,
}

impl RegionSet {
    /// The empty set.
    pub const fn empty() -> Self {
        Self { bits: 0 }
    }

    /// All detector regions ([`Region::DETECTED`]), the "all supported
    /// regions" choice.
    pub const fn all() -> Self {
        let mut set = Self::empty();
        let mut i = 0;
        while i < Region::DETECTED.len() {
            set = set.with(Region::DETECTED[i]);
            i += 1;
        }
        set
    }

    /// The set with `region` added.
    #[must_use]
    pub const fn with(self, region: Region) -> Self {
        Self {
            bits: self.bits | (1 << region.index()),
        }
    }

    /// Adds `region` to the set.
    pub const fn insert(&mut self, region: Region) {
        self.bits |= 1 << region.index();
    }

    /// Removes `region` from the set.
    pub const fn remove(&mut self, region: Region) {
        self.bits &= !(1 << region.index());
    }

    /// Whether `region` is in the set.
    pub const fn contains(&self, region: Region) -> bool {
        self.bits & (1 << region.index()) != 0
    }

    /// Whether the set has no regions.
    pub const fn is_empty(&self) -> bool {
        self.bits == 0
    }

    /// The regions in the set, in [`Region::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = Region> + '_ {
        Region::ALL.into_iter().filter(|r| self.contains(*r))
    }
}

impl fmt::Debug for RegionSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_set().entries(self.iter()).finish()
    }
}

impl FromIterator<Region> for RegionSet {
    fn from_iter<I: IntoIterator<Item = Region>>(iter: I) -> Self {
        let mut set = Self::empty();
        for region in iter {
            set.insert(region);
        }
        set
    }
}

impl From<Vec<Region>> for RegionSet {
    fn from(regions: Vec<Region>) -> Self {
        regions.into_iter().collect()
    }
}

impl From<RegionSet> for Vec<Region> {
    fn from(set: RegionSet) -> Self {
        set.iter().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{Region, RegionSet};

    #[test]
    fn all_holds_the_detector_regions_only() {
        let all = RegionSet::all();
        assert!(Region::DETECTED.iter().all(|r| all.contains(*r)));
        assert!(!all.contains(Region::Manual));
    }

    #[test]
    fn inserts_and_removes() {
        let mut set = RegionSet::empty();
        assert!(set.is_empty());
        set.insert(Region::TextChat);
        set.insert(Region::KillFeed);
        assert_eq!(
            set.iter().collect::<Vec<_>>(),
            [Region::KillFeed, Region::TextChat]
        );
        set.remove(Region::KillFeed);
        assert_eq!(set.iter().collect::<Vec<_>>(), [Region::TextChat]);
    }

    #[test]
    fn serialises_as_sorted_list() {
        let set: RegionSet = [Region::VoiceChat, Region::KillFeed].into_iter().collect();
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"["KillFeed","VoiceChat"]"#);
        assert_eq!(serde_json::from_str::<RegionSet>(&json).unwrap(), set);
    }

    #[test]
    fn keys_are_unique() {
        let mut keys: Vec<_> = Region::ALL.iter().map(|r| r.key()).collect();
        keys.sort_unstable();
        keys.dedup();
        assert_eq!(keys.len(), Region::ALL.len());
    }
}
