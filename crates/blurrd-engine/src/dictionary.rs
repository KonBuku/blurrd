//! Pass 0 of the verification: the name dictionary from the protected
//! zones.
//!
//! OCR reads the whole protected zones of the unblurred source, not only
//! the detector boxes, so names a detector missed still enter the
//! dictionary (architecture plan 7.2, step 1). The analysis reads a zone
//! at the events of its HUD structure and at least every second (see
//! `analysis::pass0`), the audit every [`DictionaryBuilder::CADENCE`]th
//! frame and on every frame where a structure signal is on; a crop is
//! OCR'd only when it changed since the zone was last read
//! ([`CropDedup`]). Each recognised line is reduced to its name part
//! with rules per zone (chat sender before the colon, no voice location
//! lines, no HUD words) before it enters the in-memory [`NameDictionary`].
//! Crops carry the pixel aspect ratio of the HUD (`sy / sx`), so OCR
//! undoes a stretched or squeezed HUD and reads every layout at the same
//! text size.
//!
//! Besides the dictionary, every read becomes a [`SourceLine`]: where the
//! text line is and where each of its name parts is, which the
//! verification compares with the blur boxes (plan 7.2, steps 2 to 5).
//! The screen surfaces' name zones are protected zones too while their
//! gate makes a frame a candidate, and a full-frame zone is read every
//! second for names outside every protected zone.
//!
//! The analysis job of the engine feeds the builder from its frame
//! pipeline on an OCR worker; the verification reads the dictionary and
//! the lines.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use blurrd_core::{HudGeometry, RectI, Region, RegionSet};
use blurrd_detect::killfeed::FEED_ROI;
use blurrd_detect::screens::{self, ScreenMap};
use blurrd_ocr::{
    CropDedup, NameDictionary, OcrEngine, OcrError, OcrRead, RgbView, Whitelist, ZoneId, ZoneSpec,
    fold,
};
use serde::Serialize;

use crate::slots::NameSlots;

/// The left HUD panel (voice chat and text chat) of plan 5.4, in reference
/// pixels of a 1920 × 1080 frame.
pub const LEFT_PANEL_ROI: RectI = RectI {
    x0: 0,
    y0: 550,
    x1: 780,
    y1: 1040,
};

/// Detection input pixels per reference pixel, whatever the source
/// resolution. HUD names are about 11 reference pixels tall; enlarged
/// twice they are found and separated from the icons next to them most
/// reliably (measured in M16 against scale 1.5, plan section 18).
pub const DET_SCALE: f32 = 1.5;

/// Detection input pixels per reference pixel of the full-frame zone: the
/// whole frame is read at the reference size, which keeps its detection
/// input near 1920 × 1088 at every resolution.
pub const FULL_FRAME_SCALE: f32 = 1.0;

/// Tallest text line of a HUD name slot, in reference rows (names are
/// about 11 rows high, their line boxes up to about 27).
pub(crate) const MAX_HUD_LINE_H: f32 = 30.0;

/// Lowest detected line that can be text OCR reads, in reference rows:
/// below it detection found specks.
pub(crate) const MIN_LINE_H: f32 = 6.0;

/// Whether a line detected in a zone of `kind` with the box `rect` (source
/// pixels, `sy` source rows per reference row) can hold a name, so it is
/// worth recognising: at least [`MIN_LINE_H`] high and wider than high,
/// and in a HUD zone no taller than [`MAX_HUD_LINE_H`]. The verification
/// takes no name from the others (it counts only HUD lines of that shape),
/// and recognition costs per line.
pub(crate) fn may_hold_name(kind: ZoneKind, rect: RectI, sy: f32) -> bool {
    let h = rect.height() as f32 / sy.max(f32::EPSILON);
    let hud = matches!(kind, ZoneKind::FeedAndBanner | ZoneKind::LeftPanel);
    h >= MIN_LINE_H && (!hud || h <= MAX_HUD_LINE_H) && rect.width() >= rect.height()
}

/// What a protected zone contains, which decides how its lines are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ZoneKind {
    /// The kill feed and the banner name strip (top right).
    FeedAndBanner,
    /// Voice chat and text chat (bottom left).
    LeftPanel,
    /// The name zone of a screen surface (read while its gate makes a
    /// frame a candidate).
    Screen(Region),
    /// The whole frame, sampled for names outside every protected zone.
    FullFrame,
}

impl ZoneKind {
    /// The zone id of each kind: the HUD zones first, then the screen
    /// surfaces in [`Region::SCREENS`] order, then the full frame.
    pub fn id(self) -> ZoneId {
        let screens = Region::SCREENS.len() as u16;
        let index = match self {
            Self::FeedAndBanner => 0,
            Self::LeftPanel => 1,
            Self::Screen(region) => {
                2 + Region::SCREENS
                    .iter()
                    .position(|&r| r == region)
                    .map_or(screens, |i| i as u16)
            }
            Self::FullFrame => 3 + screens,
        };
        ZoneId(index)
    }
}

/// A protected zone of the source frames.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProtectedZone {
    /// What it contains.
    pub kind: ZoneKind,
    /// Its OCR zone (one fixed detection size each).
    pub id: ZoneId,
    /// Where it is in the source frame, in source pixels (clipped to the
    /// frame).
    pub rect: RectI,
    /// Where it is in the 1920 × 1080 reference frame.
    pub reference: RectI,
}

/// The protected zones of a `width` × `height` source whose HUD sits at
/// `geometry` (right-anchored kill feed, left-anchored panel).
pub fn protected_zones(geometry: &HudGeometry, width: u32, height: u32) -> Vec<ProtectedZone> {
    let frame = frame_rect(width, height);
    let map = |r: RectI, x_of: &dyn Fn(i32) -> f32| {
        let y = |v: i32| geometry.oy + v as f32 * geometry.sy;
        RectI::new(
            x_of(r.x0).floor() as i32,
            y(r.y0).floor() as i32,
            x_of(r.x1).ceil() as i32,
            y(r.y1).ceil() as i32,
        )
        .ok()
        .and_then(|rect| rect.intersection(&frame))
    };
    let right = |x: i32| geometry.anchor_right - (1920 - x) as f32 * geometry.sx;
    let left = |x: i32| geometry.anchor_left + x as f32 * geometry.sx;
    [
        (
            ZoneKind::FeedAndBanner,
            ZoneId(0),
            FEED_ROI,
            map(FEED_ROI, &right),
        ),
        (
            ZoneKind::LeftPanel,
            ZoneId(1),
            LEFT_PANEL_ROI,
            map(LEFT_PANEL_ROI, &left),
        ),
    ]
    .into_iter()
    .filter_map(|(kind, id, reference, rect)| {
        rect.map(|rect| ProtectedZone {
            kind,
            id,
            rect,
            reference,
        })
    })
    .collect()
}

/// The name zones of the screen surfaces of `regions` in a `width` ×
/// `height` source whose HUD sits at `geometry`.
pub fn screen_zones(
    geometry: &HudGeometry,
    width: u32,
    height: u32,
    regions: RegionSet,
) -> Vec<ProtectedZone> {
    let map = ScreenMap::from(geometry);
    let frame = frame_rect(width, height);
    Region::SCREENS
        .iter()
        .filter(|&&region| regions.contains(region))
        .filter_map(|&region| {
            let reference = match region {
                Region::Scoreboard => screens::scoreboard::ZONE,
                Region::DeathRecap => screens::recap::ZONE,
                Region::Spectating => screens::spectating::ZONE,
                _ => screens::match_end::ZONE,
            };
            let rect = screens::zone(region, &map)?.intersection(&frame)?;
            let kind = ZoneKind::Screen(region);
            Some(ProtectedZone {
                kind,
                id: kind.id(),
                rect,
                reference,
            })
        })
        .collect()
}

/// The full-frame zone of a `width` × `height` source with the HUD scale
/// of `geometry`: the whole frame, detected at its reference size.
pub fn full_frame_zone(geometry: &HudGeometry, width: u32, height: u32) -> ProtectedZone {
    let reference = RectI {
        x0: 0,
        y0: 0,
        x1: (width as f32 / geometry.sx).round().max(1.0) as i32,
        y1: (height as f32 / geometry.sy).round().max(1.0) as i32,
    };
    ProtectedZone {
        kind: ZoneKind::FullFrame,
        id: ZoneKind::FullFrame.id(),
        rect: frame_rect(width, height),
        reference,
    }
}

/// The whole `width` × `height` frame.
fn frame_rect(width: u32, height: u32) -> RectI {
    RectI {
        x0: 0,
        y0: 0,
        x1: width as i32,
        y1: height as i32,
    }
}

/// One text line OCR found in a protected zone of the source, in source
/// pixels. Its text stays in memory, and only as far as the verification
/// needs it: the folded name parts.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceLine {
    /// The zone it was read in.
    pub zone: ZoneKind,
    /// The line's box.
    pub rect: RectI,
    /// Recognition confidence of the whole line.
    pub conf: f32,
    /// Whether the line has the shape of a chat line: a channel prefix in
    /// brackets or a colon after the sender. A text-chat line without
    /// either is part of a message.
    pub chat_shaped: bool,
    /// The parts of the line that can be player names.
    pub parts: Vec<NamePart>,
}

/// A part of a [`SourceLine`] that can be a player name.
#[derive(Debug, Clone, PartialEq)]
pub struct NamePart {
    /// Where its characters are, in source pixels (padded by a quarter of
    /// the line height on each side, inside the line's box).
    pub rect: RectI,
    /// The name, folded ([`fold`]).
    pub folded: String,
    /// Mean probability of its characters.
    pub conf: f32,
}
/// Counters of pass 0. They hold no text and nothing derived from text.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Pass0Stats {
    /// Whole frames passed to [`DictionaryBuilder::push_frame`].
    pub frames: u64,
    /// Zone crops offered.
    pub zone_crops: u64,
    /// Zone crops that were OCR'd (the rest were unchanged).
    pub zone_crops_read: u64,
    /// Text lines recognised.
    pub lines: u64,
    /// Name reads added to the dictionary.
    pub name_reads: u64,
    /// Time spent in OCR, in seconds.
    pub ocr_seconds: f64,
}

/// Builds the name dictionary from the protected zones of the source.
#[derive(Debug)]
pub struct DictionaryBuilder {
    zones: Vec<ProtectedZone>,
    /// The lines last read per zone, given again for a crop that did not
    /// change.
    last: BTreeMap<ZoneId, Arc<[SourceLine]>>,
    /// Where names can be.
    slots: NameSlots,
    /// Natural width over natural height of one source pixel of the HUD.
    pixel_aspect: f32,
    /// Source rows per reference row.
    sy: f32,
    dedup: CropDedup,
    whitelist: Whitelist,
    dictionary: NameDictionary,
    stats: Pass0Stats,
}

impl DictionaryBuilder {
    /// Zones are read every this many frames (plus every structure frame).
    pub const CADENCE: u32 = 6;

    /// A builder for a `width` × `height` source with the HUD at
    /// `geometry`, with the default [`Whitelist`].
    pub fn new(geometry: &HudGeometry, width: u32, height: u32) -> Self {
        Self::with_zones(protected_zones(geometry, width, height), geometry)
    }

    /// A builder that also reads the name zones of the screen surfaces of
    /// `regions` and the full-frame zone ([`screen_zones`],
    /// [`full_frame_zone`]), as the verification needs.
    pub fn verifying(geometry: &HudGeometry, width: u32, height: u32, regions: RegionSet) -> Self {
        let mut zones = protected_zones(geometry, width, height);
        zones.extend(screen_zones(geometry, width, height, regions));
        zones.push(full_frame_zone(geometry, width, height));
        Self::with_zones(zones, geometry)
    }

    fn with_zones(zones: Vec<ProtectedZone>, geometry: &HudGeometry) -> Self {
        Self {
            zones,
            last: BTreeMap::new(),
            slots: NameSlots::new(geometry),
            pixel_aspect: geometry.sy / geometry.sx,
            sy: geometry.sy,
            dedup: CropDedup::new(),
            whitelist: Whitelist::hud_default(),
            dictionary: NameDictionary::new(),
            stats: Pass0Stats::default(),
        }
    }

    /// Splits the zones into builders of their own, by the group
    /// `group_of` gives each zone's kind (groups without zones are
    /// dropped), so several OCR engines can read them at the same time.
    /// Each builder starts empty.
    pub(crate) fn split(self, group_of: impl Fn(ZoneKind) -> usize) -> Vec<Self> {
        let mut groups: BTreeMap<usize, Vec<ProtectedZone>> = BTreeMap::new();
        for zone in &self.zones {
            groups.entry(group_of(zone.kind)).or_default().push(*zone);
        }
        groups
            .into_values()
            .map(|zones| Self {
                zones,
                last: BTreeMap::new(),
                slots: self.slots.clone(),
                pixel_aspect: self.pixel_aspect,
                sy: self.sy,
                dedup: CropDedup::new(),
                whitelist: self.whitelist.clone(),
                dictionary: NameDictionary::new(),
                stats: Pass0Stats::default(),
            })
            .collect()
    }

    /// The zones and their detection sizes, for [`blurrd_ocr::OcrOptions`].
    /// Sizes depend only on the reference geometry, so every resolution
    /// uses the same shapes.
    pub fn zone_specs(&self) -> Vec<ZoneSpec> {
        self.zones
            .iter()
            .map(|z| {
                let scale = if z.kind == ZoneKind::FullFrame {
                    FULL_FRAME_SCALE
                } else {
                    DET_SCALE
                };
                ZoneSpec::for_crop(z.id, z.reference.width(), z.reference.height(), scale)
            })
            .collect()
    }

    /// The protected zones.
    pub fn zones(&self) -> &[ProtectedZone] {
        &self.zones
    }

    /// Whether frame `index` is read: every [`Self::CADENCE`]th frame and
    /// every frame with a structure signal on.
    pub const fn wants(index: u32, structure_on: bool) -> bool {
        structure_on || index.is_multiple_of(Self::CADENCE)
    }

    /// Reads one crop of a protected zone (the pixels of [`ProtectedZone::rect`]
    /// of a source frame) and adds the names found, unless the crop did not
    /// change since the zone was last read. Returns the zone's text lines
    /// in source pixels (for an unchanged crop, those of the last read).
    ///
    /// # Errors
    ///
    /// [`OcrError::InvalidInput`] for a zone this builder does not have,
    /// other [`OcrError`]s when OCR fails.
    pub fn push_zone(
        &mut self,
        ocr: &mut OcrEngine,
        zone: ZoneId,
        crop: RgbView<'_>,
    ) -> Result<Arc<[SourceLine]>, OcrError> {
        let mut lines = self.push_zones(ocr, &[(zone, crop)])?;
        lines
            .pop()
            .ok_or_else(|| OcrError::InvalidInput(format!("{zone} gave no lines")))
    }

    /// Reads several zone crops in order, as [`Self::push_zone`] would one
    /// after the other, but recognises the text lines of all of them
    /// together ([`OcrEngine::read_many`]). Returns the lines of each
    /// crop.
    ///
    /// # Errors
    ///
    /// As [`Self::push_zone`].
    pub fn push_zones(
        &mut self,
        ocr: &mut OcrEngine,
        crops: &[(ZoneId, RgbView<'_>)],
    ) -> Result<Vec<Arc<[SourceLine]>>, OcrError> {
        /// Where the lines of a crop come from.
        enum Source {
            /// Its own read.
            Read(usize),
            /// The lines of the zone's last read before these crops.
            Last(Arc<[SourceLine]>),
            /// Those of an earlier crop of these (an unchanged crop).
            Same(usize),
        }
        let mut sources = Vec::with_capacity(crops.len());
        let mut reads = Vec::new();
        // Per zone, the source of its latest crop so far.
        let mut latest: BTreeMap<ZoneId, usize> = BTreeMap::new();
        for (i, &(zone, crop)) in crops.iter().enumerate() {
            let (kind, rect) = self.zone(zone)?;
            self.stats.zone_crops += 1;
            let unchanged = !self.dedup.should_ocr(zone, crop);
            let source = match (unchanged, latest.get(&zone), self.last.get(&zone)) {
                (true, Some(&earlier), _) => Source::Same(earlier),
                (true, None, Some(lines)) => Source::Last(Arc::clone(lines)),
                _ => {
                    reads.push((zone, kind, rect, crop));
                    Source::Read(reads.len() - 1)
                }
            };
            latest.insert(zone, i);
            sources.push(source);
        }
        let started = Instant::now();
        let views: Vec<(ZoneId, RgbView<'_>)> = reads
            .iter()
            .map(|&(zone, _, _, crop)| (zone, crop.with_pixel_aspect(self.pixel_aspect)))
            .collect();
        let sy = self.sy;
        let keep = |crop: usize, rect: RectI| may_hold_name(reads[crop].1, rect, sy);
        let found = ocr.read_many(&views, &keep)?;
        self.stats.ocr_seconds += started.elapsed().as_secs_f64();
        self.stats.zone_crops_read += reads.len() as u64;
        let mut read_lines: Vec<Arc<[SourceLine]>> = Vec::with_capacity(reads.len());
        for (&(zone, kind, rect, _), found) in reads.iter().zip(&found) {
            let lines = self.lines_of(kind, rect, found);
            self.last.insert(zone, Arc::clone(&lines));
            read_lines.push(lines);
        }
        let mut out: Vec<Arc<[SourceLine]>> = Vec::with_capacity(crops.len());
        for source in sources {
            let lines = match source {
                Source::Read(r) => Arc::clone(&read_lines[r]),
                Source::Last(lines) => lines,
                Source::Same(earlier) => Arc::clone(&out[earlier]),
            };
            out.push(lines);
        }
        Ok(out)
    }

    /// The kind and source rectangle of `zone`.
    fn zone(&self, zone: ZoneId) -> Result<(ZoneKind, RectI), OcrError> {
        self.zones
            .iter()
            .find(|z| z.id == zone)
            .map(|z| (z.kind, z.rect))
            .ok_or_else(|| OcrError::InvalidInput(format!("{zone} is not a protected zone")))
    }

    /// The source lines of the reads `found` in a crop of a zone of
    /// `kind` at `rect`; adds the names in a HUD name slot to the
    /// dictionary.
    fn lines_of(&mut self, kind: ZoneKind, rect: RectI, found: &[OcrRead]) -> Arc<[SourceLine]> {
        self.stats.lines += found.len() as u64;
        let hud = matches!(kind, ZoneKind::FeedAndBanner | ZoneKind::LeftPanel);
        let mut lines = Vec::with_capacity(found.len());
        for read in found {
            let line = source_line(kind, (rect.x0, rect.y0), read, &self.whitelist);
            // Only names in a HUD name slot enter the dictionary; the rest
            // is matched against it.
            if hud {
                for part in &line.parts {
                    if self.slots.region_of(part.rect).is_some() {
                        self.stats.name_reads +=
                            u64::from(self.dictionary.insert(&part.folded, line.conf));
                    }
                }
            }
            lines.push(line);
        }
        lines.into()
    }

    /// Reads every protected zone of a whole source frame
    /// ([`Self::push_zone`] for each); returns the lines of every zone.
    ///
    /// # Errors
    ///
    /// [`OcrError`] when OCR fails.
    pub fn push_frame(
        &mut self,
        ocr: &mut OcrEngine,
        frame: RgbView<'_>,
    ) -> Result<Vec<Arc<[SourceLine]>>, OcrError> {
        self.count_frame();
        let zones: Vec<(ZoneId, RectI)> = self.zones.iter().map(|z| (z.id, z.rect)).collect();
        let mut lines = Vec::with_capacity(zones.len());
        for (zone, rect) in zones {
            if let Some(crop) = frame.crop(rect) {
                lines.push(self.push_zone(ocr, zone, crop)?);
            }
        }
        Ok(lines)
    }

    /// Counts a frame whose zones are pushed one by one.
    pub(crate) fn count_frame(&mut self) {
        self.stats.frames += 1;
    }

    /// The words that are never names.
    pub fn whitelist(&self) -> &Whitelist {
        &self.whitelist
    }

    /// The dictionary so far.
    pub fn dictionary(&self) -> &NameDictionary {
        &self.dictionary
    }

    /// The counters so far.
    pub fn stats(&self) -> Pass0Stats {
        self.stats
    }

    /// The dictionary and the counters.
    pub fn finish(self) -> (NameDictionary, Pass0Stats) {
        (self.dictionary, self.stats)
    }
}

/// The [`SourceLine`] of `read`, a read of a crop of zone `kind` whose
/// top-left pixel is `origin` in the source frame.
fn source_line(
    kind: ZoneKind,
    origin: (i32, i32),
    read: &OcrRead,
    whitelist: &Whitelist,
) -> SourceLine {
    let rect = RectI {
        x0: read.rect.x0 + origin.0,
        y0: read.rect.y0 + origin.1,
        x1: read.rect.x1 + origin.0,
        y1: read.rect.y1 + origin.1,
    };
    let width = rect.width() as f32;
    let pad = rect.height() as f32 * 0.25;
    let parts = name_parts(kind, &read.rec.text, whitelist)
        .into_iter()
        .filter_map(|(start, end)| {
            let (fx0, fx1) = read.rec.extent(start, end).unwrap_or((0.0, 1.0));
            let x0 = (rect.x0 as f32 + fx0 * width - pad).floor() as i32;
            let x1 = (rect.x0 as f32 + fx1 * width + pad).ceil() as i32;
            let part = RectI::new(x0.max(rect.x0), rect.y0, x1.min(rect.x1), rect.y1).ok()?;
            Some(NamePart {
                rect: part,
                folded: fold(&read.rec.text[start..end]),
                conf: read.rec.conf_of(start, end),
            })
        })
        .collect();
    let text = read.rec.text.trim();
    SourceLine {
        zone: kind,
        rect,
        conf: read.rec.conf,
        chat_shaped: text.contains(':') || strip_prefixes(text).1,
        parts,
    }
}

/// The byte ranges of a recognised line that can be player names.
///
/// - Left panel, chat line `(Channel) Name (Role): message`: the sender.
/// - Left panel, system line `(System) Name phrase`: the words before the
///   phrase ([`system_line_name`]).
/// - Left panel, voice location line (`in B Main - Team`): nothing.
/// - Anything that consists only of whitelisted words: nothing.
/// - A line longer than a name (16 folded characters) with spaces, for
///   example a killer and a victim merged into one box: each word that is
///   not whitelisted and has at least three characters; nothing in the
///   left panel, where such a line is a chat message.
pub(crate) fn name_parts(kind: ZoneKind, text: &str, whitelist: &Whitelist) -> Vec<(usize, usize)> {
    let range = |part: &str| {
        let start = part.as_ptr() as usize - text.as_ptr() as usize;
        (start, start + part.len())
    };
    let mut line = text.trim();
    if kind == ZoneKind::LeftPanel {
        let (rest, had_prefix) = strip_prefixes(line);
        line = rest;
        if let Some((sender, _message)) = line.split_once(':') {
            line = strip_trailing_group(sender.trim());
        } else if had_prefix {
            line = system_line_name(line);
        } else if is_location_line(line, whitelist) {
            return Vec::new();
        }
    }
    if line.is_empty() || whitelist.covers(line) || whitelist.contains(&fold(line)) {
        return Vec::new();
    }
    if fold(line).chars().count() <= NameDictionary::MAX_LEN || !line.contains(' ') {
        return vec![range(line)];
    }
    // A left-panel line longer than a name is a message the colon of its
    // sender was not read in; its words are no names.
    if kind == ZoneKind::LeftPanel {
        return Vec::new();
    }
    line.split_whitespace()
        .filter(|w| {
            let folded = fold(w);
            folded.chars().count() >= 3 && !whitelist.contains(&folded)
        })
        .map(range)
        .collect()
}

/// The name at the start of a system chat line (prefix removed): the words
/// before the first word of the system phrase. Phrases differ per client
/// language but start with a verb from a short list; without one, the
/// first word is taken.
fn system_line_name(line: &str) -> &str {
    /// First words of the system phrases of the supported client languages
    /// ("has abandoned the match", "hat das Match verlassen", "ha abandonado
    /// la partida", "was", "is", "joined", "left").
    const PHRASE_STARTS: [&str; 11] = [
        "has", "have", "was", "is", "joined", "left", "hat", "ist", "wurde", "ha", "se",
    ];
    let start = line.trim_start();
    let mut end = 0;
    for (i, word) in start.split_whitespace().enumerate() {
        if i > 0 && PHRASE_STARTS.contains(&fold(word).as_str()) {
            break;
        }
        end = word.as_ptr() as usize - start.as_ptr() as usize + word.len();
    }
    &start[..end]
}

/// Whether a left-panel line is a voice-chat location line: it starts with
/// a location word ("in B Main - Team"), or consists only of listed words
/// glued together after one ("inASite-Team", when OCR drops the spaces).
fn is_location_line(line: &str, whitelist: &Whitelist) -> bool {
    const LOCATION_WORDS: [&str; 3] = ["in", "en", "im"];
    if line
        .split_whitespace()
        .next()
        .is_some_and(|w| LOCATION_WORDS.contains(&fold(w).as_str()))
    {
        return true;
    }
    let folded = fold(line);
    LOCATION_WORDS.iter().any(|w| folded.starts_with(w)) && whitelist.segments(&folded)
}

/// Removes leading `(...)` and `[...]` groups (chat channel prefixes);
/// returns the rest and whether a group was removed. OCR does not always
/// read the brackets: a group without its closing bracket ends at the
/// first space, and a first word that is a channel with its opening
/// bracket read as a letter (`GALL)`, `CTeam`) or dropped (`Team)`) is a
/// group too.
fn strip_prefixes(mut line: &str) -> (&str, bool) {
    let mut removed = false;
    loop {
        let first = line.split_whitespace().next().unwrap_or_default();
        let end = match line.chars().next() {
            Some(open @ ('(' | '[')) => {
                let close = if open == '(' { ')' } else { ']' };
                line.find(close)
                    .filter(|&end| end < first.len())
                    .map_or(first.len(), |end| end + 1)
            }
            Some(_) if !removed && is_misread_channel(first) => first.len(),
            _ => return (line, removed),
        };
        line = line[end..].trim_start();
        removed = true;
    }
}

/// Whether `word`, the first word of a chat line, is a channel prefix
/// whose opening bracket OCR read as a letter or dropped.
fn is_misread_channel(word: &str) -> bool {
    /// Chat channels of the supported client languages.
    const CHANNELS: [&str; 10] = [
        "all",
        "team",
        "party",
        "system",
        "broadcast",
        "alle",
        "todos",
        "equipo",
        "sistema",
        "grupo",
    ];
    let folded = fold(word);
    let closed = word.ends_with([')', ']']);
    let after_first = folded.char_indices().nth(1).map(|(i, _)| &folded[i..]);
    CHANNELS
        .iter()
        .any(|&c| (closed && folded == c) || after_first == Some(c))
}

/// Removes a trailing `(...)` group (the role after a broadcast sender).
fn strip_trailing_group(line: &str) -> &str {
    if line.ends_with(')')
        && let Some(start) = line.rfind('(')
    {
        return line[..start].trim_end();
    }
    line
}

#[cfg(test)]
mod tests {
    use blurrd_core::{HudGeometry, HudLayout, RectI};
    use blurrd_ocr::Whitelist;

    use super::{DictionaryBuilder, ZoneKind, may_hold_name, name_parts, protected_zones};

    fn names(kind: ZoneKind, text: &str) -> Vec<String> {
        name_parts(kind, text, &Whitelist::hud_default())
            .into_iter()
            .map(|(start, end)| text[start..end].to_owned())
            .collect()
    }

    #[test]
    fn recognises_only_lines_that_can_hold_names() {
        let rect = |w: i32, h: i32| RectI::new(0, 0, w, h).unwrap();
        let screen = ZoneKind::Screen(blurrd_core::Region::MatchEnd);
        // A name line of the HUD at 1080p and, twice as tall, at 2160p.
        assert!(may_hold_name(ZoneKind::FeedAndBanner, rect(80, 18), 1.0));
        assert!(may_hold_name(ZoneKind::LeftPanel, rect(160, 36), 2.0));
        // Specks, blobs taller than a HUD line, narrow marks.
        assert!(!may_hold_name(ZoneKind::FeedAndBanner, rect(40, 5), 1.0));
        assert!(!may_hold_name(ZoneKind::LeftPanel, rect(200, 40), 1.0));
        assert!(!may_hold_name(ZoneKind::FullFrame, rect(10, 20), 1.0));
        // Screens and the full frame keep tall text (match-end names).
        assert!(may_hold_name(screen, rect(300, 48), 1.0));
        assert!(may_hold_name(ZoneKind::FullFrame, rect(300, 48), 1.0));
    }

    #[test]
    fn reads_chat_senders_and_skips_hud_text() {
        use ZoneKind::{FeedAndBanner, LeftPanel};
        assert_eq!(names(LeftPanel, "(Team) Nova Prime: go B"), ["Nova Prime"]);
        assert_eq!(names(LeftPanel, "(All) Kestrel (Caster): gg"), ["Kestrel"]);
        assert_eq!(
            names(LeftPanel, "(System) Umbra has abandoned the match"),
            ["Umbra"]
        );
        assert_eq!(
            names(LeftPanel, "(Sistema) Ngon Pham ha abandonado la partida"),
            ["Ngon Pham"]
        );
        assert_eq!(names(LeftPanel, "(System) Zed"), ["Zed"]);
        assert_eq!(names(LeftPanel, "in B Main - Team"), Vec::<String>::new());
        assert_eq!(names(LeftPanel, "in Garage Hallway"), Vec::<String>::new());
        assert_eq!(names(LeftPanel, "inASite-Team"), Vec::<String>::new());
        assert_eq!(names(LeftPanel, "Inara"), ["Inara"]);
        assert_eq!(names(LeftPanel, "Harbor Light"), ["Harbor Light"]);
        assert_eq!(names(LeftPanel, ": message only"), Vec::<String>::new());
        // Channel prefixes whose brackets OCR misread.
        assert_eq!(names(LeftPanel, "(ALL Kestrel: gg"), ["Kestrel"]);
        assert_eq!(names(LeftPanel, "GALL) Kestrel: gg"), ["Kestrel"]);
        assert_eq!(names(LeftPanel, "Team) Kestrel: gg"), ["Kestrel"]);
        assert_eq!(names(LeftPanel, "(All) Ball: hi"), ["Ball"]);
        assert_eq!(names(LeftPanel, "(ALI"), Vec::<String>::new());
        assert_eq!(
            names(LeftPanel, "(Team) Rook hacks only skills"),
            Vec::<String>::new()
        );
        assert_eq!(names(FeedAndBanner, "TEAM ACE"), Vec::<String>::new());
        assert_eq!(names(FeedAndBanner, "CLUTCH"), Vec::<String>::new());
        assert_eq!(names(FeedAndBanner, "xX_Vortex_Xx"), ["xX_Vortex_Xx"]);
        // A killer and a victim read as one line.
        assert_eq!(
            names(FeedAndBanner, "Sentinelseven Ravenclawfour"),
            ["Sentinelseven", "Ravenclawfour"]
        );
    }

    #[test]
    fn zones_follow_the_hud_geometry() {
        let reference = protected_zones(&HudGeometry::default(), 1920, 1080);
        assert_eq!(reference[0].rect, RectI::new(660, 80, 1920, 412).unwrap());
        assert_eq!(reference[1].rect, RectI::new(0, 550, 780, 1040).unwrap());
        // 1440p: 4/3 of the reference scale.
        let qhd = HudGeometry {
            sx: 4.0 / 3.0,
            sy: 4.0 / 3.0,
            anchor_right: 2560.0,
            anchor_center: 1280.0,
            ..HudGeometry::default()
        };
        let zones = protected_zones(&qhd, 2560, 1440);
        assert_eq!(zones[0].rect, RectI::new(880, 106, 2560, 550).unwrap());
        assert_eq!(zones[1].rect, RectI::new(0, 733, 1040, 1387).unwrap());
        // Pillarboxed 4:3 inside 1920 x 1080: the HUD sits between the bars.
        let pillar = HudGeometry {
            sx: 0.75,
            sy: 1.0,
            anchor_left: 240.0,
            anchor_right: 1680.0,
            anchor_center: 960.0,
            layout: HudLayout::Pillarbox { bar_px: 240 },
            ..HudGeometry::default()
        };
        let zones = protected_zones(&pillar, 1920, 1080);
        assert_eq!(zones[0].rect, RectI::new(735, 80, 1680, 412).unwrap());
        assert_eq!(zones[1].rect, RectI::new(240, 550, 825, 1040).unwrap());
    }

    #[test]
    fn detection_sizes_do_not_depend_on_the_resolution() {
        let builder = DictionaryBuilder::new(&HudGeometry::default(), 1920, 1080);
        let sizes: Vec<(u32, u32)> = builder
            .zone_specs()
            .iter()
            .map(|z| (z.width, z.height))
            .collect();
        assert_eq!(sizes, [(1920, 512), (1184, 736)]);
        assert!(DictionaryBuilder::wants(12, false));
        assert!(!DictionaryBuilder::wants(13, false));
        assert!(DictionaryBuilder::wants(13, true));
    }
}
