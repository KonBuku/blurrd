//! The PTS of every presentation frame (`pts.bin`).
//!
//! Format, little-endian:
//!
//! | Offset | Size      | Field                                     |
//! | ------ | --------- | ----------------------------------------- |
//! | 0      | 8         | magic `BLRDPTS\0`                         |
//! | 8      | 4         | format version (1)                        |
//! | 12     | 4         | frame count `n`                           |
//! | 16     | 4         | time base numerator                       |
//! | 20     | 4         | time base denominator                     |
//! | 24     | `8 × n`   | PTS of frame 0, 1, …, n − 1 (`i64`)       |
//!
//! PTS values strictly increase. A table that breaks any of these rules is
//! rejected as [`VideoError::InvalidPtsTable`].

use std::fs;
use std::io::Write;
use std::path::Path;

use blurrd_core::Rational;

use crate::VideoError;

const MAGIC: &[u8; 8] = b"BLRDPTS\0";
const VERSION: u32 = 1;
const HEADER_LEN: usize = 24;

/// PTS of every presentation frame of a video, in the video stream's time
/// base. Index `i` holds the PTS of frame `i`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtsTable {
    time_base: Rational,
    pts: Vec<i64>,
}

impl PtsTable {
    /// Creates a table.
    ///
    /// # Errors
    ///
    /// [`VideoError::Timestamps`] when the PTS values do not strictly
    /// increase or there are more than `u32::MAX` frames.
    pub fn new(time_base: Rational, pts: Vec<i64>) -> Result<Self, VideoError> {
        if u32::try_from(pts.len()).is_err() {
            return Err(VideoError::Timestamps(format!(
                "{} frames is too many",
                pts.len()
            )));
        }
        if let Some(i) = pts.windows(2).position(|w| w[1] <= w[0]) {
            return Err(VideoError::Timestamps(format!(
                "PTS of frame {} ({}) does not follow frame {} ({})",
                i + 1,
                pts[i + 1],
                i,
                pts[i]
            )));
        }
        Ok(Self { time_base, pts })
    }

    /// Time base of the PTS values.
    pub fn time_base(&self) -> Rational {
        self.time_base
    }

    /// Number of frames.
    pub fn len(&self) -> u32 {
        // `new` guarantees the length fits.
        self.pts.len() as u32
    }

    /// Whether the table has no frames.
    pub fn is_empty(&self) -> bool {
        self.pts.is_empty()
    }

    /// PTS of frame `index`, if it exists.
    pub fn pts(&self, index: u32) -> Option<i64> {
        self.pts.get(index as usize).copied()
    }

    /// All PTS values in presentation order.
    pub fn as_slice(&self) -> &[i64] {
        &self.pts
    }

    /// Presentation index of the frame with exactly this PTS.
    pub fn index_of(&self, pts: i64) -> Option<u32> {
        self.pts.binary_search(&pts).ok().map(|i| i as u32)
    }

    /// Serialises the table in the `pts.bin` format.
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(HEADER_LEN + 8 * self.pts.len());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&VERSION.to_le_bytes());
        out.extend_from_slice(&self.len().to_le_bytes());
        out.extend_from_slice(&self.time_base.num.to_le_bytes());
        out.extend_from_slice(&self.time_base.den.to_le_bytes());
        for pts in &self.pts {
            out.extend_from_slice(&pts.to_le_bytes());
        }
        out
    }

    /// Parses a table in the `pts.bin` format. `path` is only used in error
    /// messages.
    ///
    /// # Errors
    ///
    /// [`VideoError::InvalidPtsTable`] when the data is malformed.
    pub fn from_bytes(bytes: &[u8], path: &Path) -> Result<Self, VideoError> {
        let invalid = |reason: String| VideoError::InvalidPtsTable {
            path: path.to_path_buf(),
            reason,
        };
        if bytes.len() < HEADER_LEN || &bytes[..8] != MAGIC {
            return Err(invalid(String::from("not a pts.bin file")));
        }
        let word = |at: usize| {
            u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        let version = word(8);
        if version != VERSION {
            return Err(invalid(format!("unsupported version {version}")));
        }
        let count = word(12) as usize;
        let time_base = Rational::new(word(16), word(20)).map_err(|e| invalid(e.to_string()))?;
        let body = &bytes[HEADER_LEN..];
        if body.len() != count * 8 {
            return Err(invalid(format!(
                "header says {count} frames, but the file holds {} bytes of PTS",
                body.len()
            )));
        }
        let pts = body
            .as_chunks::<8>()
            .0
            .iter()
            .map(|c| i64::from_le_bytes(*c))
            .collect();
        Self::new(time_base, pts).map_err(|e| invalid(e.to_string()))
    }

    /// Reads a `pts.bin` file.
    ///
    /// # Errors
    ///
    /// [`VideoError::Io`] when the file cannot be read,
    /// [`VideoError::InvalidPtsTable`] when it is malformed.
    pub fn read(path: &Path) -> Result<Self, VideoError> {
        let bytes = fs::read(path).map_err(|source| VideoError::Io {
            context: "reading",
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_bytes(&bytes, path)
    }

    /// Writes a `pts.bin` file atomically: into `<path>.tmp` first, then
    /// renamed over `path`, so a crash never leaves a truncated table.
    ///
    /// # Errors
    ///
    /// [`VideoError::Io`] when writing or renaming fails.
    pub fn write(&self, path: &Path) -> Result<(), VideoError> {
        let io = |source| VideoError::Io {
            context: "writing",
            path: path.to_path_buf(),
            source,
        };
        let tmp = path.with_extension("bin.tmp");
        let mut file = fs::File::create(&tmp).map_err(io)?;
        file.write_all(&self.to_bytes()).map_err(io)?;
        file.sync_all().map_err(io)?;
        drop(file);
        fs::rename(&tmp, path).map_err(io)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use blurrd_core::Rational;

    use super::PtsTable;

    fn table() -> PtsTable {
        PtsTable::new(
            Rational::new(1, 15360).unwrap(),
            vec![-512, 0, 256, 768, 1000],
        )
        .unwrap()
    }

    #[test]
    fn round_trips_through_bytes_and_files() {
        let t = table();
        let bytes = t.to_bytes();
        assert_eq!(bytes.len(), 24 + 5 * 8);
        assert_eq!(PtsTable::from_bytes(&bytes, Path::new("x")).unwrap(), t);

        let dir = std::env::temp_dir().join(format!("blurrd-pts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("pts.bin");
        t.write(&path).unwrap();
        assert_eq!(PtsTable::read(&path).unwrap(), t);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn looks_up_indices() {
        let t = table();
        assert_eq!(t.len(), 5);
        assert_eq!(t.index_of(256), Some(2));
        assert_eq!(t.index_of(257), None);
        assert_eq!(t.pts(4), Some(1000));
        assert_eq!(t.pts(5), None);
    }

    #[test]
    fn rejects_malformed_tables() {
        let tb = Rational::new(1, 1000).unwrap();
        assert!(PtsTable::new(tb, vec![0, 10, 10]).is_err());
        assert!(PtsTable::new(tb, vec![5, 3]).is_err());

        let good = table().to_bytes();
        let p = Path::new("x");
        assert!(PtsTable::from_bytes(&good[..23], p).is_err());
        assert!(PtsTable::from_bytes(&good[..good.len() - 1], p).is_err());
        let mut bad_magic = good.clone();
        bad_magic[0] = b'X';
        assert!(PtsTable::from_bytes(&bad_magic, p).is_err());
        let mut bad_version = good.clone();
        bad_version[8] = 2;
        assert!(PtsTable::from_bytes(&bad_version, p).is_err());
        let mut zero_den = good;
        zero_den[20..24].copy_from_slice(&0u32.to_le_bytes());
        assert!(PtsTable::from_bytes(&zero_den, p).is_err());
    }
}
