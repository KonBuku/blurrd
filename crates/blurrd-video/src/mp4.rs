//! Edit list detection for MP4 and QuickTime files.
//!
//! FFmpeg applies edit lists while demuxing but does not report whether a
//! file has one, so the `elst` box of the first video track is read here
//! directly. Only box headers and the `moov` box are read.

use std::io::{self, Read, Seek, SeekFrom};

/// Upper bound for the `moov` box that is read into memory. Real files have
/// a few megabytes of sample tables per hour.
const MAX_MOOV: u64 = 256 << 20;

/// One box (atom) inside a buffer.
struct Atom<'a> {
    kind: [u8; 4],
    body: &'a [u8],
}

/// Iterates over the boxes in `data`. Stops at the first malformed box.
fn atoms(mut data: &[u8]) -> impl Iterator<Item = Atom<'_>> {
    std::iter::from_fn(move || {
        if data.len() < 8 {
            return None;
        }
        let size32 = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
        let kind = [data[4], data[5], data[6], data[7]];
        let (header, size) = match size32 {
            0 => (8, data.len() as u64),
            1 => {
                let large = data.get(8..16)?;
                (16, u64::from_be_bytes(large.try_into().ok()?))
            }
            n => (8, u64::from(n)),
        };
        let size = usize::try_from(size).ok()?;
        if size < header || size > data.len() {
            return None;
        }
        let body = &data[header..size];
        data = &data[size..];
        Some(Atom { kind, body })
    })
}

fn child(data: &[u8], kind: [u8; 4]) -> Option<&[u8]> {
    atoms(data)
        .find(|atom| atom.kind == kind)
        .map(|atom| atom.body)
}

/// Whether the `elst` body describes anything but the identity mapping
/// (one edit that starts at media time 0 at normal rate).
fn edit_list_shifts(elst: &[u8]) -> Option<bool> {
    let version = *elst.first()?;
    let count = u32::from_be_bytes(elst.get(4..8)?.try_into().ok()?);
    if count == 0 {
        return Some(false);
    }
    if count > 1 {
        return Some(true);
    }
    let entry = elst.get(8..)?;
    let (media_time, rate_at) = if version == 1 {
        (i64::from_be_bytes(entry.get(8..16)?.try_into().ok()?), 16)
    } else {
        (
            i64::from(i32::from_be_bytes(entry.get(4..8)?.try_into().ok()?)),
            8,
        )
    };
    let rate = u32::from_be_bytes(entry.get(rate_at..rate_at + 4)?.try_into().ok()?);
    Some(media_time != 0 || rate != 0x0001_0000)
}

/// Whether the first video track of a parsed `moov` body has an edit list
/// that shifts or trims presentation.
fn moov_has_edit_list(moov: &[u8]) -> bool {
    for trak in atoms(moov).filter(|atom| &atom.kind == b"trak") {
        let is_video = child(trak.body, *b"mdia")
            .and_then(|mdia| child(mdia, *b"hdlr"))
            .and_then(|hdlr| hdlr.get(8..12))
            .is_some_and(|handler| handler == b"vide");
        if is_video {
            return child(trak.body, *b"edts")
                .and_then(|edts| child(edts, *b"elst"))
                .and_then(edit_list_shifts)
                .unwrap_or(false);
        }
    }
    false
}

/// Reads the top-level boxes of an MP4/QuickTime stream and reports whether
/// its first video track has an edit list that shifts or trims
/// presentation. Returns `Ok(false)` for files without a `moov` box.
pub(crate) fn has_edit_list(reader: &mut (impl Read + Seek)) -> io::Result<bool> {
    let end = reader.seek(SeekFrom::End(0))?;
    let mut pos = 0;
    while pos + 8 <= end {
        reader.seek(SeekFrom::Start(pos))?;
        let mut header = [0u8; 16];
        reader.read_exact(&mut header[..8])?;
        let size32 = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);
        let (header_len, size) = match size32 {
            0 => (8, end - pos),
            1 => {
                reader.read_exact(&mut header[8..16])?;
                let mut large = [0u8; 8];
                large.copy_from_slice(&header[8..16]);
                (16, u64::from_be_bytes(large))
            }
            n => (8, u64::from(n)),
        };
        if size < header_len || pos + size > end {
            return Ok(false);
        }
        if &header[4..8] == b"moov" {
            let body_len = size - header_len;
            if body_len > MAX_MOOV {
                return Ok(false);
            }
            let mut body = vec![0u8; body_len as usize];
            reader.read_exact(&mut body)?;
            return Ok(moov_has_edit_list(&body));
        }
        pos += size;
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use super::has_edit_list;

    fn atom(kind: [u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = ((body.len() + 8) as u32).to_be_bytes().to_vec();
        out.extend_from_slice(&kind);
        out.extend_from_slice(body);
        out
    }

    fn hdlr(handler: [u8; 4]) -> Vec<u8> {
        let mut body = vec![0u8; 8];
        body.extend_from_slice(&handler);
        body.extend_from_slice(&[0u8; 12]);
        atom(*b"hdlr", &body)
    }

    /// An `elst` with version-0 entries `(duration, media_time, rate)`.
    fn elst(entries: &[(u32, i32, u32)]) -> Vec<u8> {
        let mut body = vec![0u8; 4];
        body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (duration, media_time, rate) in entries {
            body.extend_from_slice(&duration.to_be_bytes());
            body.extend_from_slice(&media_time.to_be_bytes());
            body.extend_from_slice(&rate.to_be_bytes());
        }
        atom(*b"elst", &body)
    }

    fn file(tracks: &[(&[u8; 4], Option<Vec<u8>>)]) -> Vec<u8> {
        let mut moov = Vec::new();
        for (handler, edit) in tracks {
            let mut trak = Vec::new();
            if let Some(elst) = edit {
                trak.extend(atom(*b"edts", elst));
            }
            trak.extend(atom(*b"mdia", &hdlr(**handler)));
            moov.extend(atom(*b"trak", &trak));
        }
        let mut out = atom(*b"ftyp", b"isom\0\0\0\0");
        out.extend(atom(*b"mdat", &[0u8; 32]));
        out.extend(atom(*b"moov", &moov));
        out
    }

    fn check(data: Vec<u8>) -> bool {
        has_edit_list(&mut Cursor::new(data)).unwrap()
    }

    #[test]
    fn identity_edit_is_not_an_edit_list() {
        assert!(!check(file(&[(
            b"vide",
            Some(elst(&[(1000, 0, 0x10000)]))
        )])));
        assert!(!check(file(&[(b"vide", None)])));
    }

    #[test]
    fn shifting_trimming_or_empty_edits_are_detected() {
        assert!(check(file(&[(
            b"vide",
            Some(elst(&[(1000, 1024, 0x10000)]))
        )])));
        assert!(check(file(&[(
            b"vide",
            Some(elst(&[(10, -1, 0x10000), (1000, 0, 0x10000)]))
        )])));
    }

    #[test]
    fn only_the_video_track_counts() {
        let audio_edit = Some(elst(&[(1000, 2048, 0x10000)]));
        assert!(!check(file(&[
            (b"soun", audio_edit.clone()),
            (b"vide", None)
        ])));
        assert!(check(file(&[(b"soun", None), (b"vide", audio_edit)])));
    }

    #[test]
    fn non_mp4_and_truncated_data_report_no_edit_list() {
        assert!(!check(b"\x1aE\xdf\xa3 matroska".to_vec()));
        let mut truncated = file(&[(b"vide", Some(elst(&[(1000, 1024, 0x10000)])))]);
        truncated.truncate(truncated.len() - 5);
        assert!(!check(truncated));
    }
}
