//! Dense proposals for building goldens (architecture plan 11.2).
//!
//! A golden must not be built from the detector alone, or it would inherit
//! the detector's misses. Proposals are therefore the union of four
//! sources: detector boxes, OCR text-detection boxes over the whole
//! protected zones, slot layout boxes wherever a structure signal is on,
//! and pixel changes between frames inside the zones. Overlapping boxes of
//! one frame are merged, and merged boxes are linked over time into
//! proposal items that a reviewer then accepts, corrects or rejects.

use blurrd_core::{FrameRange, RectI};

use crate::image::RgbFrame;

/// Where a proposal box came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum ProposalSource {
    /// A detector box.
    Detector,
    /// An OCR text-detection box.
    OcrText,
    /// A slot box of a region whose structure signal is on.
    SlotLayout,
    /// Pixels that changed since the previous frame.
    PixelChange,
}

impl ProposalSource {
    const fn bit(self) -> u8 {
        match self {
            Self::Detector => 1,
            Self::OcrText => 2,
            Self::SlotLayout => 4,
            Self::PixelChange => 8,
        }
    }
}

/// A set of [`ProposalSource`]s.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct SourceSet(u8);

impl SourceSet {
    /// Adds a source.
    pub const fn insert(&mut self, source: ProposalSource) {
        self.0 |= source.bit();
    }

    /// Whether the set contains `source`.
    pub const fn contains(self, source: ProposalSource) -> bool {
        self.0 & source.bit() != 0
    }

    /// Number of sources in the set.
    pub const fn len(self) -> u32 {
        self.0.count_ones()
    }

    /// Whether the set is empty.
    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

/// A proposed item: one box per frame over a frame range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// Frames the item was proposed in (gaps up to the link tolerance are
    /// bridged and have no key).
    pub frames: FrameRange,
    /// The merged box of every frame with a key, in frame order.
    pub keys: Vec<(u32, RectI)>,
    /// Every source that contributed.
    pub sources: SourceSet,
}

impl Proposal {
    /// The union of all keys.
    pub fn extent(&self) -> Option<RectI> {
        self.keys.iter().map(|(_, r)| *r).reduce(union)
    }
}

fn union(a: RectI, b: RectI) -> RectI {
    RectI {
        x0: a.x0.min(b.x0),
        y0: a.y0.min(b.y0),
        x1: a.x1.max(b.x1),
        y1: a.y1.max(b.y1),
    }
}

fn grown(r: RectI, by: i32) -> RectI {
    RectI {
        x0: r.x0 - by,
        y0: r.y0 - by,
        x1: r.x1 + by,
        y1: r.y1 + by,
    }
}

/// Builds proposals frame by frame.
#[derive(Debug, Clone)]
pub struct ProposalBuilder {
    /// Boxes closer than this many pixels are merged within a frame.
    merge_gap: i32,
    /// Smallest `IoU` that links a box to an item of the previous frames.
    link_iou: f32,
    /// Frames an item may miss before it ends.
    max_gap: u32,
    active: Vec<Proposal>,
    done: Vec<Proposal>,
    last_frame: Option<u32>,
}

impl Default for ProposalBuilder {
    /// Merges boxes within 2 px, links at `IoU` ≥ 0.3 across gaps of up to
    /// 2 frames.
    fn default() -> Self {
        Self::new(2, 0.3, 2)
    }
}

impl ProposalBuilder {
    /// A builder with explicit tolerances.
    pub const fn new(merge_gap: i32, link_iou: f32, max_gap: u32) -> Self {
        Self {
            merge_gap,
            link_iou,
            max_gap,
            active: Vec::new(),
            done: Vec::new(),
            last_frame: None,
        }
    }

    /// Adds every box of `frame`. Frames must arrive in increasing order;
    /// an earlier or repeated frame is ignored.
    pub fn push_frame(&mut self, frame: u32, boxes: &[(ProposalSource, RectI)]) {
        if self.last_frame.is_some_and(|f| frame <= f) {
            return;
        }
        self.last_frame = Some(frame);
        // Items that missed more than `max_gap` frames have ended.
        let max_gap = self.max_gap;
        let (keep, ended): (Vec<_>, Vec<_>) = std::mem::take(&mut self.active)
            .into_iter()
            .partition(|p| frame - p.frames.f1 <= max_gap + 1);
        self.active = keep;
        self.done.extend(ended);
        let merged = self.merge(boxes);
        let mut taken = vec![false; self.active.len()];
        for (rect, sources) in merged {
            let best = self
                .active
                .iter()
                .enumerate()
                .filter(|(i, _)| !taken[*i])
                .filter_map(|(i, p)| {
                    let last = p.keys.last().map(|(_, r)| *r)?;
                    let iou = last.iou(&rect);
                    (iou >= self.link_iou).then_some((i, iou))
                })
                .max_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((i, _)) = best {
                taken[i] = true;
                let p = &mut self.active[i];
                p.frames.f1 = frame;
                p.keys.push((frame, rect));
                p.sources = p.sources.union(sources);
            } else {
                taken.push(true);
                self.active.push(Proposal {
                    frames: FrameRange {
                        f0: frame,
                        f1: frame,
                    },
                    keys: vec![(frame, rect)],
                    sources,
                });
            }
        }
    }

    /// Merges overlapping or touching boxes of one frame.
    fn merge(&self, boxes: &[(ProposalSource, RectI)]) -> Vec<(RectI, SourceSet)> {
        let mut groups: Vec<(RectI, SourceSet)> = Vec::new();
        for &(source, rect) in boxes {
            let mut set = SourceSet::default();
            set.insert(source);
            let mut current = (rect, set);
            // Absorb every group the box (or what it grew into) touches.
            loop {
                let hit = groups.iter().position(|(g, _)| {
                    grown(*g, self.merge_gap).intersection(&current.0).is_some()
                });
                match hit {
                    Some(i) => {
                        let (g, s) = groups.swap_remove(i);
                        current = (union(g, current.0), s.union(current.1));
                    }
                    None => break,
                }
            }
            groups.push(current);
        }
        groups.sort_by_key(|(r, _)| (r.y0, r.x0));
        groups
    }

    /// Ends every item and returns all proposals ordered by first frame.
    pub fn finish(mut self) -> Vec<Proposal> {
        self.done.append(&mut self.active);
        self.done
            .sort_by_key(|p| (p.frames.f0, p.keys.first().map(|(_, r)| (r.y0, r.x0))));
        self.done
    }
}

/// Boxes around the pixels of `zone` whose colour changed by more than
/// `threshold` (in any channel) between `previous` and `current`.
///
/// Changed pixels separated by at most `merge_gap` unchanged pixels (in
/// any direction) are grouped, so the strokes of one word become one box. Boxes smaller than
/// `min_pixels` changed pixels are dropped (noise).
pub fn pixel_change_boxes(
    previous: &RgbFrame,
    current: &RgbFrame,
    zone: RectI,
    threshold: u8,
    merge_gap: i32,
    min_pixels: u32,
) -> Vec<RectI> {
    let Some(zone) = zone
        .intersection(&previous.bounds())
        .and_then(|z| z.intersection(&current.bounds()))
    else {
        return Vec::new();
    };
    let (w, h) = (zone.width() as usize, zone.height() as usize);
    let changed: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = (zone.x0 + (i % w) as i32, zone.y0 + (i / w) as i32);
            match (previous.pixel(x, y), current.pixel(x, y)) {
                (Some(a), Some(b)) => (0..3).any(|c| a[c].abs_diff(b[c]) > threshold),
                _ => false,
            }
        })
        .collect();
    // Label connected groups of changed pixels; pixels up to `merge_gap`
    // unchanged pixels apart are neighbours.
    let mut label = vec![u32::MAX; w * h];
    let mut out = Vec::new();
    let mut stack = Vec::new();
    let gap = merge_gap.max(0) + 1;
    for start in 0..w * h {
        if !changed[start] || label[start] != u32::MAX {
            continue;
        }
        let id = out.len() as u32;
        label[start] = id;
        stack.push(start);
        let (mut x0, mut y0, mut x1, mut y1, mut n) = (w, h, 0, 0, 0u32);
        while let Some(i) = stack.pop() {
            let (x, y) = (i % w, i / w);
            n += 1;
            x0 = x0.min(x);
            y0 = y0.min(y);
            x1 = x1.max(x + 1);
            y1 = y1.max(y + 1);
            for dy in -gap..=gap {
                for dx in -gap..=gap {
                    let (nx, ny) = (x as i32 + dx, y as i32 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i32 || ny >= h as i32 {
                        continue;
                    }
                    let j = ny as usize * w + nx as usize;
                    if changed[j] && label[j] == u32::MAX {
                        label[j] = id;
                        stack.push(j);
                    }
                }
            }
        }
        out.push((n, x0, y0, x1, y1));
    }
    out.into_iter()
        .filter(|(n, ..)| *n >= min_pixels)
        .map(|(_, x0, y0, x1, y1)| RectI {
            x0: zone.x0 + x0 as i32,
            y0: zone.y0 + y0 as i32,
            x1: zone.x0 + x1 as i32,
            y1: zone.y0 + y1 as i32,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use blurrd_core::RectI;

    use super::{ProposalBuilder, ProposalSource, pixel_change_boxes};
    use crate::image::RgbFrame;

    fn rect(x0: i32, y0: i32, x1: i32, y1: i32) -> RectI {
        RectI::new(x0, y0, x1, y1).unwrap()
    }

    #[test]
    fn merges_sources_and_links_frames() {
        let mut b = ProposalBuilder::default();
        b.push_frame(
            10,
            &[
                (ProposalSource::Detector, rect(100, 100, 150, 120)),
                (ProposalSource::OcrText, rect(148, 101, 160, 119)),
                (ProposalSource::SlotLayout, rect(300, 100, 400, 120)),
            ],
        );
        b.push_frame(
            11,
            &[(ProposalSource::PixelChange, rect(101, 100, 160, 120))],
        );
        // Frame 12 is missed, frame 13 continues within the gap.
        b.push_frame(13, &[(ProposalSource::Detector, rect(100, 100, 160, 120))]);
        b.push_frame(20, &[(ProposalSource::Detector, rect(100, 100, 160, 120))]);
        let p = b.finish();
        assert_eq!(p.len(), 3);
        let first = &p[0];
        assert_eq!((first.frames.f0, first.frames.f1), (10, 13));
        assert_eq!(first.keys.len(), 3);
        assert_eq!(first.keys[0].1, rect(100, 100, 160, 120));
        for s in [
            ProposalSource::Detector,
            ProposalSource::OcrText,
            ProposalSource::PixelChange,
        ] {
            assert!(first.sources.contains(s));
        }
        assert!(!first.sources.contains(ProposalSource::SlotLayout));
        assert_eq!(p[1].sources.len(), 1);
        assert_eq!((p[2].frames.f0, p[2].frames.f1), (20, 20));
        assert_eq!(p[0].extent(), Some(rect(100, 100, 160, 120)));
    }

    #[test]
    fn ignores_out_of_order_frames() {
        let mut b = ProposalBuilder::default();
        b.push_frame(5, &[(ProposalSource::Detector, rect(0, 0, 10, 10))]);
        b.push_frame(5, &[(ProposalSource::Detector, rect(50, 50, 60, 60))]);
        assert_eq!(b.finish().len(), 1);
    }

    #[test]
    fn finds_changed_words() {
        let a = RgbFrame::filled(64, 32, [50, 50, 50]);
        let mut b = a.clone();
        // Two strokes 2 px apart form one word; a lone pixel is noise.
        b.fill_rect(rect(10, 10, 12, 20), [250, 250, 250], 1.0);
        b.fill_rect(rect(14, 10, 16, 20), [250, 250, 250], 1.0);
        b.set(40, 5, [255, 0, 0]);
        let boxes = pixel_change_boxes(&a, &b, a.bounds(), 30, 2, 4);
        assert_eq!(boxes, [rect(10, 10, 16, 20)]);
        assert_eq!(pixel_change_boxes(&a, &a, a.bounds(), 30, 2, 1), []);
    }
}
