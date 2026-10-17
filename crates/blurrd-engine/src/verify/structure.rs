//! The HUD structure signals an analysis saw, per frame, kept in the
//! cache (`structure.bin`) for the jobs that read the source again
//! without the detectors: the audit of an output and the verification of
//! edited ranges.
//!
//! The verification confirms a name slot only where its region showed
//! its structure. Reading the source again without these signals would
//! have to treat every region as showing its structure, so world text,
//! callouts, the buy menu or scoreboard labels under an idle zone would
//! be confirmed as names there although the analysis' verification
//! (correctly) skipped them.
//!
//! The file holds a magic, the video's frame count and runs of frames
//! with the same regions on: `f0`, `f1` and a mask over
//! [`Region::DETECTED`], all little-endian `u32`.

use std::path::Path;

use blurrd_core::{AppError, Region, RegionSet};

use super::LineLog;
use crate::cache;

/// The file's magic and format version.
const MAGIC: &[u8; 8] = b"BLRDSTR1";

/// The regions with structure per frame of a video.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StructureLog {
    /// The video's frames.
    frames: u32,
    /// Runs of frames `(f0, f1, regions)` in frame order, without gaps
    /// for frames with structure (frames outside every run showed none).
    runs: Vec<(u32, u32, RegionSet)>,
}

impl StructureLog {
    /// The structure `log` recorded for a video of `frames` frames.
    pub(crate) fn from_lines(log: &LineLog, frames: u32) -> Self {
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
        Self { frames, runs }
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

    /// Writes the log to `path` atomically (a temporary file, then a
    /// rename).
    ///
    /// # Errors
    ///
    /// [`blurrd_core::ErrorKind::Io`] when the file cannot be written.
    pub(crate) fn write(&self, path: &Path) -> Result<(), AppError> {
        let mut bytes = Vec::with_capacity(12 + self.runs.len() * 12);
        bytes.extend_from_slice(MAGIC);
        bytes.extend_from_slice(&self.frames.to_le_bytes());
        for &(f0, f1, regions) in &self.runs {
            bytes.extend_from_slice(&f0.to_le_bytes());
            bytes.extend_from_slice(&f1.to_le_bytes());
            bytes.extend_from_slice(&mask(regions).to_le_bytes());
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
    /// frames.
    pub(crate) fn read(path: &Path, frames: u32) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let (head, body) = bytes.split_at_checked(12)?;
        let (magic, count) = head.split_at(8);
        if magic != MAGIC || u32::from_le_bytes(count.try_into().ok()?) != frames {
            return None;
        }
        let (chunks, rest) = body.as_chunks::<12>();
        if !rest.is_empty() {
            return None;
        }
        let mut runs = Vec::with_capacity(chunks.len());
        for run in chunks {
            let word = |i: usize| u32::from_le_bytes([run[i], run[i + 1], run[i + 2], run[i + 3]]);
            let (f0, f1, bits) = (word(0), word(4), word(8));
            let ordered = runs.last().is_none_or(|&(_, last, _)| last < f0);
            if f0 > f1 || f1 >= frames || !ordered {
                return None;
            }
            runs.push((f0, f1, unmask(bits)));
        }
        Some(Self { frames, runs })
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

    use super::{LineLog, StructureLog};
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
        let structure = StructureLog::from_lines(&log, 10);
        assert_eq!(structure.runs, [(3, 5, feed), (6, 6, both), (9, 9, feed)]);
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
        assert_eq!(read, Some(structure));
        assert_eq!(other, None);
        assert_eq!(StructureLog::read(&path, 10), None);
    }
}
