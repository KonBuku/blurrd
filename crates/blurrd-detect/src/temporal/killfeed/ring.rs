//! The frames the kill-feed tracker keeps: a fixed ring of feed strips.

use blurrd_core::RectI;

use crate::detection::Detection;
use crate::image::{GrayImage, RgbView};
use crate::killfeed::FEED_ROI;

use super::STRIP;
use super::follow::Frame;

/// What the tracker keeps of one frame.
#[derive(Debug, Clone, Default)]
pub(super) struct Slot {
    /// Frame index, or `None` for a slot that holds no frame yet.
    pub(super) frame: Option<u32>,
    /// `min(R, G, B)` of the feed strip ([`STRIP`]), the plane names are
    /// correlated on.
    pub(super) mn: GrayImage,
    /// The kill-feed detections of the frame.
    pub(super) detections: Vec<Detection>,
    /// Their text bounds.
    pub(super) names: Vec<RectI>,
}

impl Slot {
    /// Heap bytes held by the slot.
    fn heap_bytes(&self) -> usize {
        self.mn.data().len()
            + self.detections.capacity() * size_of::<Detection>()
            + self.names.capacity() * size_of::<RectI>()
    }
}

/// A ring of the last `capacity` frames. Slots are reused, so the memory
/// does not grow with the length of the video.
#[derive(Debug, Clone)]
pub(super) struct Ring {
    slots: Vec<Slot>,
}

impl Ring {
    /// A ring of `capacity` empty slots.
    pub(super) fn new(capacity: usize) -> Self {
        Self {
            slots: vec![Slot::default(); capacity],
        }
    }

    /// Stores `frame`, replacing the oldest frame.
    pub(super) fn store(&mut self, frame: u32, crop: &RgbView<'_>, detections: &[Detection]) {
        let index = self.index(frame);
        let slot = &mut self.slots[index];
        slot.frame = Some(frame);
        let (w, h) = (STRIP.width(), STRIP.height());
        slot.mn.resize(w, h);
        let x0 = 3 * (STRIP.x0 - FEED_ROI.x0) as usize;
        let row_bytes = 3 * w as usize;
        for (y, dst) in (0..h).zip(slot.mn.data_mut().chunks_exact_mut(w as usize)) {
            let src = &crop.row(y)[x0..x0 + row_bytes];
            for (d, px) in dst.iter_mut().zip(src.as_chunks::<3>().0) {
                *d = px[0].min(px[1]).min(px[2]);
            }
        }
        slot.detections.clear();
        slot.detections.extend_from_slice(detections);
        slot.names.clear();
        slot.names.extend(detections.iter().map(|d| d.text));
    }

    /// What a follower reads of frame `frame`, if the ring still holds it.
    pub(super) fn frame(&self, frame: u32) -> Option<Frame<'_>> {
        self.get(frame).map(|s| Frame {
            index: frame,
            plane: &s.mn,
            names: &s.names,
        })
    }

    /// The stored frame `frame`, if the ring still holds it.
    pub(super) fn get(&self, frame: u32) -> Option<&Slot> {
        let slot = &self.slots[self.index(frame)];
        (slot.frame == Some(frame)).then_some(slot)
    }

    /// Heap bytes held by the ring.
    pub(super) fn heap_bytes(&self) -> usize {
        self.slots.capacity() * size_of::<Slot>()
            + self.slots.iter().map(Slot::heap_bytes).sum::<usize>()
    }

    fn index(&self, frame: u32) -> usize {
        frame as usize % self.slots.len()
    }
}
