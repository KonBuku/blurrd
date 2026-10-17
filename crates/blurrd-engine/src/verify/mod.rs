//! Verify and fix: checks that no name stays readable in the blurred
//! output and covers every one that does (architecture plan 7.2).
//!
//! Pass 0 ([`crate::dictionary`]) reads the protected zones of the
//! unblurred source during the analysis and leaves a [`LineLog`]: every
//! text line it found, with the parts that can be player names. The blur
//! only ever removes information, so a name readable in the output was
//! readable in the source; the verification therefore starts from the
//! source lines:
//!
//! 1. **Coverage.** Every name part in a name slot of every frame pass 0
//!    read (the frames with HUD structure and one frame in
//!    [`crate::DictionaryBuilder::CADENCE`] elsewhere) is compared with the
//!    blur boxes the plan has on that frame. A slot counts on frames where
//!    its region shows its structure and the detectors boxed a name of it
//!    within a second (a buy menu over the kill feed has no such boxes); a
//!    screen surface's slots are the rows of its boxes. A part is
//!    *exposed* when a run of its columns at least half a line high (about
//!    one glyph) is not covered. The dictionary of this check holds the
//!    names read in the slots of such frames.
//! 2. **Leak classes.** The blurred pixels of every exposed part are made
//!    exactly as the render will make them (the blur is local, so
//!    blurring a crop of the frame gives the output's pixels) and read
//!    again; [`LeakMatcher`] decides whether the read still shows a name
//!    (dictionary match, or confident text outside the dictionary in a
//!    name slot). Lines of chat messages, voice location lines and HUD
//!    words never form name parts.
//! 3. **Fix.** A readable run gets an `OcrFix` track: a slot box around the
//!    exposed parts from the frame after the last frame read without it to
//!    the frame before the next one, checked again on the same frames and
//!    grown up to three times.
//! 4. **Positive control.** Per region, the detector's items (tracks) with
//!    text under them on a read frame count as verified when OCR read that
//!    text on the source; the others are unverifiable.
//! 5. **Out of scope.** The full-frame samples (every half second) list
//!    dictionary names outside every protected zone as information (D8);
//!    names in a screen surface's name zone whose gate stayed off count as
//!    exposed parts of that surface.
//!
//! Reports hold frames, regions, boxes, confidences and counts, never text
//! or values derived from it.

mod pixels;
mod scan;

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use blurrd_core::{
    AppError, BlurPlan, Confidence, FrameBoxTable, FrameRange, HudGeometry, Leak, LeakKind,
    LeakStatus, LeakZone, NameKind, RectI, Region, RegionSet, RegionVerify, Segment, Track,
    TrackId, TrackSource, VerifyReport, schema,
};
use blurrd_ocr::{LeakMatcher, NameDictionary, NameMatch, Whitelist};

pub(crate) use self::pixels::{PixelSetup, Pixels};
use self::scan::{Run, Select};
use crate::cache;
use crate::dictionary::{DictionaryBuilder, SourceLine, ZoneKind};
use crate::job::{Job, internal};
use crate::slots::NameSlots;

/// The most frames of one exposed run whose blurred pixels are read; runs
/// that are longer are read at evenly spaced frames (always the first and
/// the last).
const MAX_READS_PER_RUN: usize = 24;

/// How often a fix box grows before a leak is reported open.
const FIX_ROUNDS: usize = 3;

/// The source lines pass 0 read, per frame. They stay in memory.
#[derive(Debug, Clone, Default)]
pub(crate) struct LineLog {
    frames: BTreeMap<u32, FrameLines>,
    /// Frames in half a second of the video.
    half_second: u32,
}

/// The lines of one frame, per zone read.
#[derive(Debug, Clone, Default)]
pub(crate) struct FrameLines {
    /// Whether the HUD zones were read (the frame counts as checked).
    pub(crate) hud: bool,
    /// The HUD regions whose structure the frame showed: only their name
    /// slots hold names on it.
    pub(crate) structure: RegionSet,
    /// The lines of every zone read.
    pub(crate) zones: Vec<Arc<[SourceLine]>>,
}

impl FrameLines {
    /// The lines of the zones other than the full frame.
    fn protected(&self) -> impl Iterator<Item = &SourceLine> {
        self.zones
            .iter()
            .flat_map(|z| z.iter())
            .filter(|l| l.zone != ZoneKind::FullFrame)
    }

    /// The lines of the full-frame sample.
    fn full_frame(&self) -> impl Iterator<Item = &SourceLine> {
        self.zones
            .iter()
            .flat_map(|z| z.iter())
            .filter(|l| l.zone == ZoneKind::FullFrame)
    }
}

impl LineLog {
    /// An empty log of a video with `half_second` frames in half a
    /// second.
    pub(crate) fn new(half_second: u32) -> Self {
        Self {
            frames: BTreeMap::new(),
            half_second: half_second.max(1),
        }
    }

    /// Records the lines read on `frame`, where the regions `structure`
    /// showed their HUD structure (frames in any order; a frame recorded
    /// twice keeps both sets).
    pub(crate) fn record(
        &mut self,
        frame: u32,
        hud: bool,
        structure: RegionSet,
        zones: Vec<Arc<[SourceLine]>>,
    ) {
        let entry = self.frames.entry(frame).or_default();
        entry.hud |= hud;
        for region in structure.iter() {
            entry.structure.insert(region);
        }
        entry.zones.extend(zones);
    }

    /// Frames in half a second of the video.
    pub(crate) fn half_second(&self) -> u32 {
        self.half_second
    }

    /// Every recorded frame and its lines, in frame order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (u32, &FrameLines)> {
        self.frames.iter().map(|(&f, l)| (f, l))
    }

    /// The recorded frames in `range`.
    fn range(&self, f0: u32, f1: u32) -> impl Iterator<Item = (u32, &FrameLines)> {
        self.frames.range(f0..=f1).map(|(&f, l)| (f, l))
    }

    /// The last frame before `frame` whose HUD zones were read.
    fn checked_before(&self, frame: u32) -> Option<u32> {
        self.frames
            .range(..frame)
            .rev()
            .find(|(_, l)| l.hud)
            .map(|(&f, _)| f)
    }

    /// The first frame after `frame` whose HUD zones were read.
    fn checked_after(&self, frame: u32) -> Option<u32> {
        self.frames
            .range(frame.saturating_add(1)..)
            .find(|(_, l)| l.hud)
            .map(|(&f, _)| f)
    }
}

/// What the verification of one video reads.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Context<'a> {
    /// The lines pass 0 read.
    pub(crate) log: &'a LineLog,
    /// Words that are never names.
    pub(crate) whitelist: &'a Whitelist,
    /// Where the HUD is.
    pub(crate) geometry: &'a HudGeometry,
    /// Frame size.
    pub(crate) size: (u32, u32),
    /// The regions that are blurred.
    pub(crate) regions: RegionSet,
}

/// The result of a verification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Outcome {
    /// `OcrFix` tracks to add to the project.
    pub(crate) fixes: Vec<Track>,
    /// Leaks (fixed, open) and out-of-scope names, in frame order.
    pub(crate) leaks: Vec<Leak>,
    /// Counts per region.
    pub(crate) per_region: BTreeMap<Region, RegionVerify>,
    /// Frames whose protected zones were read.
    pub(crate) frames_checked: u32,
}

/// Verifies `plan` over the frames of `ctx.log` and fixes what it can,
/// reading blurred pixels through `pixels`. `frames` limits the
/// verification to these frame ranges (all frames when `None`). Emits a
/// `LeakFound` event per leak.
///
/// # Errors
///
/// Decoding, blur and OCR errors; a cancellation of `job`.
pub(crate) fn verify(
    ctx: &Context<'_>,
    plan: &BlurPlan,
    pixels: &mut Pixels<'_>,
    frames: Option<&[FrameRange]>,
    job: &Job<'_>,
) -> Result<Outcome, AppError> {
    let checks = Checks::new(ctx, plan);
    let matcher = LeakMatcher::new(&checks.dictionary, ctx.whitelist);
    let inside = |f: u32| frames.is_none_or(|ranges| ranges.iter().any(|r| r.contains(f)));
    let scan = checks.scan(ctx, &matcher, Select::Exposed, &inside);
    let mut plan = plan.clone();
    let mut next_id = plan.tracks.iter().map(|t| t.id.0).max().unwrap_or(0) + 1;
    let mut fixes = Vec::new();
    let mut leaks = Vec::new();
    for run in &scan.runs {
        if job.is_canceled() {
            return Err(crate::job::canceled());
        }
        let Some(found) = read_run(run, &plan, pixels, &matcher)? else {
            continue;
        };
        let range = fix_range(ctx.log, run, plan.frame_count);
        let mut status = LeakStatus::Open;
        let mut track = None;
        for round in 0..FIX_ROUNDS {
            let fix = fix_track(TrackId(next_id), run, range, round, ctx);
            plan.tracks.retain(|t| t.id != fix.id);
            plan.tracks.push(fix.clone());
            track = Some(fix);
            if still_readable(&found.frames, run, &plan, pixels, &matcher, ctx.regions)?.is_none() {
                status = LeakStatus::Fixed;
                break;
            }
        }
        if let Some(track) = track {
            fixes.push(track);
            next_id += 1;
        }
        job.leak(run.first(), run.region, status);
        leaks.push(leak(run, &found, status));
    }
    leaks.extend(scan.out_of_scope);
    leaks.sort_by_key(|l| (l.frame_range.f0, l.frame_range.f1));
    Ok(Outcome {
        fixes,
        per_region: positive_control(ctx, &plan, &inside),
        leaks,
        frames_checked: ctx.log.iter().filter(|(f, l)| l.hud && inside(*f)).count() as u32,
    })
}

/// Where each region has boxes, which confirm its name slots: a slot
/// holds a name of its region on a frame where the region shows its
/// structure and has a box within a window around the frame; for the kill
/// feed and the chat, whose rows lie in a tall zone over the game world, a
/// box on the same row (a sign on a wall behind the feed is on none).
#[derive(Debug, Clone, Default)]
pub(crate) struct Presence {
    /// Per region: frame, top and bottom of every box, in frame order.
    boxes: BTreeMap<Region, Vec<(u32, i32, i32)>>,
    window: u32,
    /// Source rows per reference row.
    sy: f32,
}

impl Presence {
    /// The boxes of the regions of `table` (a plan of `frame_count`
    /// frames of a video with the HUD scale `sy`), confirming within
    /// `window` frames.
    pub(crate) fn new(table: &FrameBoxTable, frame_count: u32, window: u32, sy: f32) -> Self {
        let mut boxes: BTreeMap<Region, Vec<(u32, i32, i32)>> = BTreeMap::new();
        for frame in 0..frame_count {
            for b in table.boxes(frame) {
                boxes
                    .entry(b.region)
                    .or_default()
                    .push((frame, b.rect.y0, b.rect.y1));
            }
        }
        Self { boxes, window, sy }
    }

    /// Whether a box of `region` within the window around `frame`
    /// confirms a name at `rect` (on its row for the kill feed and the
    /// chat).
    pub(crate) fn confirms(&self, region: Region, frame: u32, rect: RectI) -> bool {
        let Some(boxes) = self.boxes.get(&region) else {
            return false;
        };
        let rows = matches!(region, Region::KillFeed | Region::TextChat);
        let (centre, half) = (rect.y0 + rect.y1, rect.height() as i32 / 2);
        let from = frame.saturating_sub(self.window);
        let to = frame.saturating_add(self.window);
        boxes[boxes.partition_point(|b| b.0 < from)..]
            .iter()
            .take_while(|b| b.0 <= to)
            .any(|&(_, y0, y1)| !rows || (2 * (y0 - half) <= centre && centre < 2 * (y1 + half)))
    }
}

/// Frames a name must be read on to stay in the verification's
/// dictionary: a name on screen is read on many frames, the text of a menu
/// fading in over the HUD on one or two.
const MIN_NAME_READS: u32 = 3;

/// The names read in the HUD name slots that `presence` confirms, on at
/// least [`MIN_NAME_READS`] frames, with three letters and digits or more
/// (game names have three to sixteen) and a letter: names of a kill feed,
/// banner, voice list or chat the detectors found, not HUD numbers or the
/// text of a menu over them.
fn confirmed_names(log: &LineLog, slots: &NameSlots, presence: &Presence) -> NameDictionary {
    let mut dictionary = NameDictionary::new();
    for (frame, lines) in log.iter() {
        for line in lines.protected() {
            if !matches!(line.zone, ZoneKind::FeedAndBanner | ZoneKind::LeftPanel) {
                continue;
            }
            for part in &line.parts {
                let confirmed = slots.region_of(part.rect).is_some_and(|region| {
                    holds_name(region, line, frame, lines.structure, presence)
                });
                if confirmed && plausible_name(&part.folded) {
                    dictionary.insert(&part.folded, line.conf);
                }
            }
        }
    }
    dictionary.retain_read(MIN_NAME_READS);
    dictionary
}

/// Tallest text line of a HUD name slot, in reference rows (names are
/// about 11 rows high, their line boxes up to about 27).
const MAX_SLOT_LINE_H: f32 = 30.0;

/// Whether `line`, in a name slot of `region` on `frame`, can hold a name
/// of the region: the region shows its structure (`structure`) and
/// `presence` confirms the slot; the line is no taller than a HUD line
/// (with the HUD scale `sy` folded into `presence`); a text-chat line has
/// the shape of a chat line.
pub(crate) fn holds_name(
    region: Region,
    line: &SourceLine,
    frame: u32,
    structure: RegionSet,
    presence: &Presence,
) -> bool {
    structure.contains(region)
        && line.rect.height() as f32 <= MAX_SLOT_LINE_H * presence.sy
        && (region != Region::TextChat || line.chat_shaped)
        && presence.confirms(region, frame, line.rect)
}

/// Whether a folded name part can be a game name: three letters and
/// digits or more, at least one of them a letter.
pub(crate) fn plausible_name(folded: &str) -> bool {
    folded.chars().count() >= 3 && folded.chars().any(char::is_alphabetic)
}

/// Reads the output's pixels of every name part of the frames of
/// `ctx.log` with `pixels` (which reads a rendered output: no blur is
/// added) and lists every part still readable as an open leak, without
/// fixing anything: the audit of an output (plan 7.2 step 8). Emits a
/// `LeakFound` event per leak.
///
/// # Errors
///
/// Decoding and OCR errors; a cancellation of `job`.
pub(crate) fn audit(
    ctx: &Context<'_>,
    plan: &BlurPlan,
    pixels: &mut Pixels<'_>,
    job: &Job<'_>,
) -> Result<Outcome, AppError> {
    let checks = Checks::new(ctx, plan);
    let matcher = LeakMatcher::new(&checks.dictionary, ctx.whitelist);
    let scan = checks.scan(ctx, &matcher, Select::All, &|_| true);
    let mut leaks = Vec::new();
    for run in &scan.runs {
        if job.is_canceled() {
            return Err(crate::job::canceled());
        }
        if let Some(found) = read_run(run, plan, pixels, &matcher)? {
            job.leak(run.first(), run.region, LeakStatus::Open);
            leaks.push(leak(run, &found, LeakStatus::Open));
        }
    }
    leaks.extend(scan.out_of_scope);
    leaks.sort_by_key(|l| (l.frame_range.f0, l.frame_range.f1));
    Ok(Outcome {
        fixes: Vec::new(),
        per_region: positive_control(ctx, plan, &|_| true),
        leaks,
        frames_checked: ctx.log.iter().filter(|(_, l)| l.hud).count() as u32,
    })
}

/// What the checks of one plan over one line log share.
struct Checks {
    table: FrameBoxTable,
    slots: NameSlots,
    presence: Presence,
    /// The confirmed names ([`confirmed_names`]).
    dictionary: NameDictionary,
}

impl Checks {
    fn new(ctx: &Context<'_>, plan: &BlurPlan) -> Self {
        let table = plan.frame_table(&ctx.regions);
        let slots = NameSlots::new(ctx.geometry);
        let presence = Presence::new(
            &table,
            plan.frame_count,
            2 * ctx.log.half_second(),
            ctx.geometry.sy,
        );
        let dictionary = confirmed_names(ctx.log, &slots, &presence);
        Self {
            table,
            slots,
            presence,
            dictionary,
        }
    }

    /// The name parts `select` picks on the frames `inside` holds for,
    /// in runs, and the out-of-scope names.
    fn scan(
        &self,
        ctx: &Context<'_>,
        matcher: &LeakMatcher<'_>,
        select: Select,
        inside: &dyn Fn(u32) -> bool,
    ) -> scan::Scan {
        scan::scan(
            ctx,
            &scan::Places::new(ctx, self.slots.clone(), &self.table, &self.presence),
            matcher,
            select,
            inside,
        )
    }
}

/// The leak entry of `run`, found readable as `found` says.
fn leak(run: &Run, found: &Found, status: LeakStatus) -> Leak {
    Leak {
        frame_range: FrameRange {
            f0: run.first(),
            f1: run.last(),
        },
        region: Some(run.region),
        zone: LeakZone::NameSlot,
        rect: run.union(),
        conf: found.conf,
        kind: match found.kind {
            NameMatch::Dictionary => LeakKind::Dictionary,
            NameMatch::DictionaryFree => LeakKind::DictionaryFree,
        },
        status,
        dedup: None,
    }
}

/// What made a run a leak: the frames whose blurred pixels were readable,
/// the strongest class and the best confidence.
struct Found {
    frames: Vec<usize>,
    kind: NameMatch,
    conf: f32,
}

/// Reads the blurred pixels of the exposed parts of `run` (at most
/// [`MAX_READS_PER_RUN`] of them); `None` when none is readable.
fn read_run(
    run: &Run,
    plan: &BlurPlan,
    pixels: &mut Pixels<'_>,
    matcher: &LeakMatcher<'_>,
) -> Result<Option<Found>, AppError> {
    let mut found: Option<Found> = None;
    let mut boxes = Vec::new();
    for index in spread(run.exposures.len(), MAX_READS_PER_RUN) {
        let exposure = &run.exposures[index];
        plan.boxes_at(exposure.frame, &pixels.regions(), &mut boxes);
        if let Some((kind, conf)) = pixels.read(exposure, &boxes, matcher)? {
            let entry = found.get_or_insert(Found {
                frames: Vec::new(),
                kind,
                conf,
            });
            entry.frames.push(index);
            if kind == NameMatch::Dictionary {
                entry.kind = kind;
            }
            entry.conf = entry.conf.max(conf);
        }
    }
    Ok(found)
}

/// Reads the exposures `indices` of `run` again with the boxes of `plan`;
/// the first that is still readable, if any.
fn still_readable(
    indices: &[usize],
    run: &Run,
    plan: &BlurPlan,
    pixels: &mut Pixels<'_>,
    matcher: &LeakMatcher<'_>,
    regions: RegionSet,
) -> Result<Option<usize>, AppError> {
    let mut boxes = Vec::new();
    for &index in indices {
        let exposure = &run.exposures[index];
        plan.boxes_at(exposure.frame, &regions, &mut boxes);
        if pixels.read(exposure, &boxes, matcher)?.is_some() {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

/// Up to `max` indices spread evenly over `0..len`, first and last
/// included.
fn spread(len: usize, max: usize) -> Vec<usize> {
    if len <= max {
        return (0..len).collect();
    }
    let mut out: Vec<usize> = (0..max)
        .map(|i| (i * (len - 1)).div_ceil(max - 1))
        .collect();
    out.dedup();
    out
}

/// The frames a fix of `run` covers. A run of the HUD zones (read on
/// every frame with structure) reaches from the frame after the last frame
/// read without the run's exposure to the frame before the next one, at
/// most a reading interval beyond the run; a run of the full-frame samples
/// (one frame in half a second) reaches half its sampling gap beyond the
/// run on both sides, as nothing is known between two samples.
fn fix_range(log: &LineLog, run: &Run, frame_count: u32) -> (u32, u32) {
    let (first, last) = (run.first(), run.last());
    let last_frame = frame_count.saturating_sub(1);
    if run.spacing > DictionaryBuilder::CADENCE {
        let reach = (run.spacing / 2).saturating_sub(1);
        return (
            first.saturating_sub(reach),
            last.saturating_add(reach).min(last_frame).max(last),
        );
    }
    let gap = run.spacing.saturating_sub(1);
    let f0 = log
        .checked_before(first)
        .map_or(first.saturating_sub(gap), |f| f + 1)
        .max(first.saturating_sub(gap));
    let f1 = log
        .checked_after(last)
        .map_or(last.saturating_add(gap), |f| f - 1)
        .min(last.saturating_add(gap))
        .min(last_frame);
    (f0.min(first), f1.max(last))
}

/// The `OcrFix` track of `run` over `range` in fix round `round`: the
/// union of the exposed parts padded around (more each round; from the
/// second round the whole lines), as one slot box.
fn fix_track(id: TrackId, run: &Run, range: (u32, u32), round: usize, ctx: &Context<'_>) -> Track {
    const PAD_Y: [f32; FIX_ROUNDS] = [0.3, 0.6, 1.0];
    const PAD_X: [f32; FIX_ROUNDS] = [0.6, 1.2, 2.0];
    let base = if round == 0 {
        run.union()
    } else {
        run.line_union()
    };
    let height = run.height() as f32;
    let aspect = ctx.geometry.sx / ctx.geometry.sy;
    let (pad_x, pad_y) = (
        (PAD_X[round] * height * aspect).ceil() as i32,
        (PAD_Y[round] * height).ceil() as i32,
    );
    let (w, h) = (ctx.size.0 as i32, ctx.size.1 as i32);
    let rect = RectI {
        x0: (base.x0 - pad_x).max(0),
        y0: (base.y0 - pad_y).max(0),
        x1: (base.x1 + pad_x).min(w),
        y1: (base.y1 + pad_y).min(h),
    };
    Track {
        id,
        region: run.region,
        kind: NameKind::OcrFix,
        source: TrackSource::OcrFix,
        segments: vec![Segment {
            f0: range.0,
            f1: range.1,
            rect,
            conf: Confidence::OcrFix,
            text_offset: (0, 0),
        }],
    }
}

/// The positive control per region: a detector track with text under its
/// box on a read frame is an item; it is verified when OCR read that text
/// with the leak threshold's confidence on at least one frame.
fn positive_control(
    ctx: &Context<'_>,
    plan: &BlurPlan,
    inside: &dyn Fn(u32) -> bool,
) -> BTreeMap<Region, RegionVerify> {
    let mut per_region: BTreeMap<Region, (u32, u32)> = ctx
        .regions
        .iter()
        .filter(|&r| r != Region::Manual)
        .map(|r| (r, (0, 0)))
        .collect();
    for track in &plan.tracks {
        if track.source != TrackSource::Detector {
            continue;
        }
        let Some(counts) = per_region.get_mut(&track.region) else {
            continue;
        };
        let (mut text, mut read) = (false, false);
        'segments: for segment in &track.segments {
            for (frame, lines) in ctx.log.range(segment.f0, segment.f1) {
                if !inside(frame) {
                    continue;
                }
                for line in lines.protected() {
                    if !text_shaped(line.rect, ctx.geometry.sy)
                        || !covers_text(segment.rect, line.rect)
                    {
                        continue;
                    }
                    text = true;
                    if line.conf >= LeakMatcher::FREE_CONF {
                        read = true;
                        break 'segments;
                    }
                }
            }
        }
        if text {
            counts.0 += 1;
            counts.1 += u32::from(read);
        }
    }
    per_region
        .into_iter()
        .map(|(region, (items, read))| {
            (
                region,
                RegionVerify {
                    positive_control: if items == 0 {
                        1.0
                    } else {
                        read as f32 / items as f32
                    },
                    items,
                    unverifiable_items: items - read,
                },
            )
        })
        .collect()
}

/// Whether a detected line at `rect` has the shape of a line of HUD text
/// (`sy` source rows per reference row): between 6 and 30 reference rows
/// high and wider than high. Detection also returns specks and large
/// blobs on fading panels and textures, which are no text OCR could read.
fn text_shaped(rect: RectI, sy: f32) -> bool {
    let h = rect.height() as f32 / sy.max(f32::EPSILON);
    (6.0..=MAX_SLOT_LINE_H).contains(&h) && rect.width() >= rect.height()
}

/// Whether box `rect` lies over text line `line`: at least 30 % of the
/// smaller of the two is shared.
fn covers_text(rect: RectI, line: RectI) -> bool {
    rect.intersection(&line).is_some_and(|i| {
        let smaller = rect.area().min(line.area()).max(1);
        i.area() * 10 >= smaller * 3
    })
}

/// The smallest rectangle containing `a` and `b`.
pub(crate) fn union(a: RectI, b: RectI) -> RectI {
    RectI {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

/// Builds the report of `outcome` for project revision `revision` of the
/// video with `fingerprint`.
pub(crate) fn report(
    outcome: &Outcome,
    revision: u64,
    fingerprint: blurrd_core::Fingerprint,
    coverage: blurrd_core::Coverage,
) -> VerifyReport {
    VerifyReport::new(
        revision,
        fingerprint,
        coverage,
        outcome.frames_checked,
        outcome.per_region.clone(),
        outcome.leaks.clone(),
    )
}

/// The file name of the report of project revision `revision`.
pub(crate) fn report_name(revision: u64) -> String {
    format!("verify-{revision}.json")
}

/// Writes `report` to `path` atomically (a temporary file, then a
/// rename).
///
/// # Errors
///
/// [`blurrd_core::ErrorKind::Io`] when the file cannot be written; an
/// internal error for a report that does not validate.
pub(crate) fn write_report(report: &VerifyReport, path: &Path) -> Result<(), AppError> {
    let text = schema::to_json(report).map_err(|e| internal(format!("verify report: {e}")))?;
    let mut temp = path.as_os_str().to_owned();
    temp.push(".tmp");
    let temp = std::path::PathBuf::from(temp);
    std::fs::write(&temp, text)
        .and_then(|()| std::fs::rename(&temp, path))
        .map_err(|e| {
            let _ = std::fs::remove_file(&temp);
            cache::io_error(&e, "The verification report could not be saved.", path)
        })
}

/// The warnings a finished verification gives: out-of-scope names and
/// unverifiable items.
pub(crate) fn warn(outcome: &Outcome, job: &Job<'_>) {
    let out_of_scope = outcome
        .leaks
        .iter()
        .filter(|l| l.kind == LeakKind::OutOfScope)
        .count();
    if out_of_scope > 0 {
        job.warning(
            blurrd_core::WarningCode::OutOfScopeNames,
            format!(
                "{out_of_scope} name(s) seen outside every supported region (for example name \
                 tags above heads); they are out of scope and not blurred."
            ),
            None,
        );
    }
    let unverifiable: u32 = outcome
        .per_region
        .values()
        .map(|r| r.unverifiable_items)
        .sum();
    if unverifiable > 0 {
        job.warning(
            blurrd_core::WarningCode::UnverifiableItems,
            format!(
                "OCR could not read {unverifiable} name(s) on the source; they are covered only by \
                 the detector boxes."
            ),
            None,
        );
    }
}

#[cfg(test)]
mod tests {
    use blurrd_core::{
        BlurPlan, Confidence, NameKind, RectI, Region, RegionSet, Segment, Track, TrackId,
        TrackSource,
    };

    use super::{LineLog, Presence, covers_text, spread};

    #[test]
    fn presence_confirms_slots_near_boxes() {
        let track = Track {
            id: TrackId(1),
            region: Region::KillFeed,
            kind: NameKind::Victim,
            source: TrackSource::Detector,
            segments: vec![Segment {
                f0: 100,
                f1: 110,
                rect: RectI::new(1700, 100, 1800, 120).unwrap(),
                conf: Confidence::Tracked,
                text_offset: (0, 0),
            }],
        };
        let plan = BlurPlan {
            frame_count: 400,
            tracks: vec![track],
            overrides: Vec::new(),
        };
        let presence = Presence::new(&plan.frame_table(&RegionSet::all()), 400, 60, 1.0);
        let row = RectI::new(1600, 102, 1660, 118).unwrap();
        assert!(presence.confirms(Region::KillFeed, 105, row));
        assert!(presence.confirms(Region::KillFeed, 40, row));
        assert!(presence.confirms(Region::KillFeed, 170, row));
        assert!(!presence.confirms(Region::KillFeed, 171, row));
        assert!(!presence.confirms(Region::KillFeed, 39, row));
        assert!(!presence.confirms(Region::TextChat, 105, row));
        // A sign below the feed's rows is on none of them.
        let sign = RectI::new(1300, 355, 1500, 390).unwrap();
        assert!(!presence.confirms(Region::KillFeed, 105, sign));
    }
    use crate::dictionary::{SourceLine, ZoneKind};

    #[test]
    fn spreads_reads_over_long_runs() {
        assert_eq!(spread(3, 24), [0, 1, 2]);
        let picked = spread(100, 5);
        assert_eq!(picked.first(), Some(&0));
        assert_eq!(picked.last(), Some(&99));
        assert_eq!(picked.len(), 5);
    }

    #[test]
    fn boxes_over_lines() {
        let line = RectI::new(100, 100, 200, 120).unwrap();
        assert!(covers_text(RectI::new(90, 98, 210, 122).unwrap(), line));
        assert!(covers_text(RectI::new(150, 100, 160, 120).unwrap(), line));
        assert!(!covers_text(RectI::new(195, 100, 260, 120).unwrap(), line));
    }

    #[test]
    fn finds_the_checked_frames_around_a_run() {
        let mut log = LineLog::new(30);
        let line: std::sync::Arc<[SourceLine]> = vec![SourceLine {
            zone: ZoneKind::FeedAndBanner,
            rect: RectI::new(0, 0, 10, 10).unwrap(),
            conf: 0.9,
            chat_shaped: false,
            parts: Vec::new(),
        }]
        .into();
        for f in [0, 6, 12, 13, 14, 30] {
            log.record(f, f != 30, RegionSet::all(), vec![line.clone()]);
        }
        assert_eq!(log.checked_before(12), Some(6));
        assert_eq!(log.checked_after(14), None);
        assert_eq!(log.checked_after(13), Some(14));
    }
}
