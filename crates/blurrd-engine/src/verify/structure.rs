//! The HUD structure signals an analysis saw and the zones its pass 0
//! read, per frame, kept in the cache (`structure.bin`) for the jobs that
//! read the source again without the detectors: the audit of an output
//! and the verification of edited ranges.
//!
//! The verification confirms a name slot only where its region showed
//! its structure. Reading the source again without these signals would
//! have to treat every region as showing its structure, so world text,
//! callouts, the buy menu or scoreboard labels under an idle zone would
//! be confirmed as names there although the analysis' verification
//! (correctly) skipped them. Pass 0 of an analysis reads each zone at the
//! events of its HUD structure, which only the detectors see; the audit
//! reads the same zones of the same frames, so it checks what the
//! analysis' verification checked, on the encoded output.
//!
//! The file holds, all little-endian `u32` after an 8-byte magic: the
//! video's frame count, the number of runs, the frames between two reads
//! of a HUD zone that join one name's exposures, the runs of frames with
//! the same regions on (`f0`, `f1` and a mask over [`Region::DETECTED`])
//! and the frames pass 0 read with the zones it read (`frame` and a mask
//! made by [`read_mask`]). A file of the first version holds the frame
//! count and the runs only.

use std::path::Path;

use blurrd_core::{AppError, Region, RegionSet};

use super::{HudRead, LineLog};
use crate::analysis::pass0::ZoneReads;
use crate::cache;

/// The file's magic and format version.
const MAGIC: &[u8; 8] = b"BLRDSTR2";

/// The magic of the first version (the structure runs only).
const MAGIC_V1: &[u8; 8] = b"BLRDSTR1";

/// The regions with structure per frame of a video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureLog {
    /// The video's frames.
    frames: u32,
    /// Runs of frames `(f0, f1, regions)` in frame order, without gaps
    /// for frames with structure (frames outside every run showed none).
    runs: Vec<(u32, u32, RegionSet)>,
    /// What pass 0 read, when the file records it.
    reads: Option<ReadLog>,
}

/// The zones pass 0 of an analysis read.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ReadLog {
    /// Frames between two reads of a HUD zone that join the exposures of
    /// one name ([`LineLog::hud_spacing`]).
    hud_spacing: u32,
    /// The frames read and their zones, in frame order.
    frames: Vec<(u32, ZoneReads)>,
}

impl StructureLog {
    /// The structure `log` recorded for a video of `frames` frames, and
    /// the zones pass 0 read per frame (`scheduled`, in frame order).
    pub(crate) fn from_lines(log: &LineLog, scheduled: &[(u32, ZoneReads)], frames: u32) -> Self {
        let mut runs: Vec<(u32, u32, RegionSet)> = Vec::new();
        for (frame, lines) in log.iter() {
            if lines.structure.is_empty() || frame >= frames {
                continue;
            }
            match runs.last_mut() {
                Some(last) if last.1 + 1 == frame && last.2 == lines.structure => last.1 = frame,
                _ => runs.push((frame, frame, lines.structure)),
            }
        }
        let reads = ReadLog {
            hud_spacing: log.hud_spacing(),
            frames: scheduled
                .iter()
                .filter(|(frame, _)| *frame < frames)
                .copied()
                .collect(),
        };
        Self {
            frames,
            runs,
            reads: Some(reads),
        }
    }

    /// The regions that showed their structure on `frame`.
    pub(crate) fn at(&self, frame: u32) -> RegionSet {
        let after = self.runs.partition_point(|r| r.0 <= frame);
        after
            .checked_sub(1)
            .and_then(|i| self.runs.get(i))
            .filter(|r| frame <= r.1)
            .map_or(RegionSet::empty(), |r| r.2)
    }

    /// The zones pass 0 of the analysis read on `frame` (none when it read
    /// nothing there); `None` when the file does not record them.
    pub(crate) fn reads_at(&self, frame: u32) -> Option<ZoneReads> {
        let reads = self.reads.as_ref()?;
        Some(
            reads
                .frames
                .binary_search_by_key(&frame, |r| r.0)
                .map_or(ZoneReads::default(), |i| reads.frames[i].1),
        )
    }

    /// The frames between two reads of a HUD zone that joined the
    /// exposures of one name in the analysis, when the file records it.
    pub(crate) fn hud_spacing(&self) -> Option<u32> {
        self.reads.as_ref().map(|r| r.hud_spacing)
    }

    /// Writes the log to `path` atomically (a temporary file, then a
    /// rename).
    ///
    /// # Errors
    ///
    /// [`blurrd_core::ErrorKind::Io`] when the file cannot be written.
    pub(crate) fn write(&self, path: &Path) -> Result<(), AppError> {
        let (hud_spacing, reads) = self
            .reads
            .as_ref()
            .map_or((1, &[][..]), |r| (r.hud_spacing, r.frames.as_slice()));
        let mut words = Vec::with_capacity(3 + self.runs.len() * 3 + reads.len() * 2);
        words.push(self.frames);
        words.push(u32::try_from(self.runs.len()).unwrap_or(u32::MAX));
        words.push(hud_spacing);
        for &(f0, f1, regions) in &self.runs {
            words.extend([f0, f1, mask(regions)]);
        }
        for &(frame, read) in reads {
            words.extend([frame, read_mask(read)]);
        }
        let mut bytes = Vec::with_capacity(MAGIC.len() + words.len() * 4);
        bytes.extend_from_slice(MAGIC);
        for word in words {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        let temp = path.with_extension("bin.tmp");
        std::fs::write(&temp, bytes)
            .and_then(|()| std::fs::rename(&temp, path))
            .map_err(|e| {
                let _ = std::fs::remove_file(&temp);
                cache::io_error(&e, "The structure signals could not be saved.", path)
            })
    }

    /// The log at `path` when it is a valid log of a video of `frames`
    /// frames (of either version).
    pub(crate) fn read(path: &Path, frames: u32) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let (magic, body) = bytes.split_at_checked(MAGIC.len())?;
        let (words, rest) = body.as_chunks::<4>();
        if !rest.is_empty() {
            return None;
        }
        let words: Vec<u32> = words.iter().map(|w| u32::from_le_bytes(*w)).collect();
        let (&count, words) = words.split_first()?;
        if count != frames {
            return None;
        }
        let (runs, reads) = if magic == MAGIC {
            let (&[runs, hud_spacing], words) = words.split_first_chunk::<2>()?;
            let len = usize::try_from(runs).ok()?.checked_mul(3)?;
            let (runs, reads) = words.split_at_checked(len)?;
            (runs, Some((hud_spacing, reads)))
        } else if magic == MAGIC_V1 {
            (words, None)
        } else {
            return None;
        };
        let (runs, rest) = runs.as_chunks::<3>();
        if !rest.is_empty() {
            return None;
        }
        let mut log = Self {
            frames,
            runs: Vec::with_capacity(runs.len()),
            reads: None,
        };
        for &[f0, f1, bits] in runs {
            let ordered = log.runs.last().is_none_or(|&(_, last, _)| last < f0);
            if f0 > f1 || f1 >= frames || !ordered {
                return None;
            }
            log.runs.push((f0, f1, unmask(bits)));
        }
        if let Some((hud_spacing, reads)) = reads {
            let (reads, rest) = reads.as_chunks::<2>();
            if !rest.is_empty() || hud_spacing == 0 {
                return None;
            }
            let mut read_log = ReadLog {
                hud_spacing,
                frames: Vec::with_capacity(reads.len()),
            };
            for &[frame, bits] in reads {
                let ordered = read_log.frames.last().is_none_or(|&(last, _)| last < frame);
                if frame >= frames || !ordered {
                    return None;
                }
                read_log.frames.push((frame, read_unmask(bits)));
            }
            log.reads = Some(read_log);
        }
        Some(log)
    }
}

/// `regions` as a mask over [`Region::DETECTED`].
fn mask(regions: RegionSet) -> u32 {
    Region::DETECTED
        .iter()
        .enumerate()
        .filter(|(_, r)| regions.contains(**r))
        .fold(0, |m, (i, _)| m | (1 << i))
}

/// `reads` as a mask: bit 0 the kill feed and banner, bit 1 the left
/// panel, bits 2 to 5 the screen surfaces in [`Region::SCREENS`] order,
/// bit 6 the full frame.
fn read_mask(reads: ZoneReads) -> u32 {
    [reads.hud.feed, reads.hud.left]
        .into_iter()
        .chain(reads.screens)
        .chain([reads.full])
        .enumerate()
        .fold(0, |m, (i, on)| if on { m | (1 << i) } else { m })
}

/// The zones of a mask made by [`read_mask`].
fn read_unmask(bits: u32) -> ZoneReads {
    let on = |i: usize| bits & (1 << i) != 0;
    ZoneReads {
        hud: HudRead {
            feed: on(0),
            left: on(1),
        },
        screens: std::array::from_fn(|i| on(2 + i)),
        full: on(6),
    }
}

/// The regions of a mask over [`Region::DETECTED`].
fn unmask(bits: u32) -> RegionSet {
    Region::DETECTED
        .iter()
        .enumerate()
        .filter(|(i, _)| bits & (1 << i) != 0)
        .map(|(_, r)| *r)
        .collect()
}

#[cfg(test)]
mod tests {
    use blurrd_core::{Region, RegionSet};

    use super::{LineLog, MAGIC_V1, StructureLog};
    use crate::analysis::pass0::ZoneReads;
    use crate::verify::HudRead;

    #[test]
    fn runs_round_trip_and_answer_per_frame() {
        let feed = RegionSet::empty().with(Region::KillFeed);
        let both = feed.with(Region::Banner);
        let mut log = LineLog::new(30);
        for frame in 0..10 {
            log.record(frame, HudRead::BOTH, RegionSet::empty(), Vec::new());
        }
        for frame in 3..6 {
            log.record(frame, HudRead::BOTH, feed, Vec::new());
        }
        log.record(6, HudRead::BOTH, both, Vec::new());
        log.record(9, HudRead::BOTH, feed, Vec::new());
        log.set_hud_spacing(60);
        let left = ZoneReads {
            hud: HudRead {
                feed: false,
                left: true,
            },
            screens: [false, true, false, false],
            full: false,
        };
        let scheduled = [(0, ZoneReads::ALL), (4, left), (12, ZoneReads::ALL)];
        let structure = StructureLog::from_lines(&log, &scheduled, 10);
        assert_eq!(structure.runs, [(3, 5, feed), (6, 6, both), (9, 9, feed)]);
        // The zones read, per frame; frames past the video are dropped.
        assert_eq!(structure.reads_at(0), Some(ZoneReads::ALL));
        assert_eq!(structure.reads_at(4), Some(left));
        assert_eq!(structure.reads_at(5), Some(ZoneReads::default()));
        assert_eq!(structure.reads_at(12), Some(ZoneReads::default()));
        assert_eq!(structure.hud_spacing(), Some(60));
        assert_eq!(structure.at(0), RegionSet::empty());
        assert_eq!(structure.at(4), feed);
        assert_eq!(structure.at(6), both);
        assert_eq!(structure.at(7), RegionSet::empty());
        assert_eq!(structure.at(9), feed);

        let path =
            std::env::temp_dir().join(format!("blurrd-structure-{}.bin", std::process::id()));
        structure.write(&path).unwrap();
        let read = StructureLog::read(&path, 10);
        // A log of another video is not used.
        let other = StructureLog::read(&path, 11);
        std::fs::remove_file(&path).unwrap();
        assert_eq!(read, Some(structure.clone()));
        assert_eq!(other, None);
        assert_eq!(StructureLog::read(&path, 10), None);

        // A file of the first version gives the runs without the reads.
        let mut bytes = MAGIC_V1.to_vec();
        for word in [10u32, 3, 5, 0b1] {
            bytes.extend_from_slice(&word.to_le_bytes());
        }
        std::fs::write(&path, bytes).unwrap();
        let v1 = StructureLog::read(&path, 10).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert_eq!(
            v1.at(4),
            RegionSet::empty().with(super::Region::DETECTED[0])
        );
        assert_eq!(v1.reads_at(0), None);
        assert_eq!(v1.hud_spacing(), None);
    }
}
