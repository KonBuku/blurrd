//! URLs the preview player loads videos from (architecture plan 9.2).
//!
//! Videos play through Tauri's asset protocol, whose scope holds exactly
//! the files the user chose (see [`crate::paths`]); the preview proxy in
//! the cache is added to the scope when its URL is handed out. Videos
//! larger than [`LARGE_VIDEO_BYTES`] play from the loopback media server
//! instead, when it runs.

use std::fmt::Write as _;
use std::path::Path;

use crate::media_server::MediaServer;

/// Size from which a video plays from the loopback server: the asset
/// protocol hands every range through the webview's resource callback
/// and holds its body in memory, which makes seeking in large files slow.
pub(crate) const LARGE_VIDEO_BYTES: u64 = 1 << 30;

/// The URL of the video `path` (`size` bytes) of project `id`: the
/// loopback server for large files when it runs, the asset protocol
/// otherwise.
pub(crate) fn video_url(server: Option<&MediaServer>, id: &str, path: &Path, size: u64) -> String {
    match server {
        Some(server) if size >= LARGE_VIDEO_BYTES => server.url(id, path),
        _ => asset_url(path),
    }
}

/// The asset protocol URL of `path`, as the frontend's `convertFileSrc`
/// builds it: `http://asset.localhost/<path>` on Windows,
/// `asset://localhost/<path>` elsewhere, with the path percent-encoded
/// like `encodeURIComponent` does.
pub(crate) fn asset_url(path: &Path) -> String {
    let base = if cfg!(windows) {
        "http://asset.localhost/"
    } else {
        "asset://localhost/"
    };
    format!("{base}{}", encode_component(&path.to_string_lossy()))
}

/// `text` percent-encoded like JavaScript's `encodeURIComponent`: every
/// byte of its UTF-8 form except `A-Z a-z 0-9 - _ . ! ~ * ' ( )`.
pub(crate) fn encode_component(text: &str) -> String {
    let mut encoded = String::with_capacity(text.len());
    for &byte in text.as_bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{LARGE_VIDEO_BYTES, asset_url, encode_component, video_url};
    use crate::media_server::MediaServer;

    #[test]
    fn components_encode_like_the_browser() {
        assert_eq!(
            encode_component(r"C:\clips\a b (1).mp4"),
            "C%3A%5Cclips%5Ca%20b%20(1).mp4"
        );
        assert_eq!(encode_component("ü"), "%C3%BC");
    }

    #[cfg(windows)]
    #[test]
    fn asset_urls_use_the_windows_host() {
        assert_eq!(
            asset_url(Path::new(r"C:\a.mp4")),
            "http://asset.localhost/C%3A%5Ca.mp4"
        );
    }

    #[test]
    fn large_videos_play_from_the_loopback_server() {
        let server = MediaServer::start().unwrap();
        let path = Path::new(r"C:\clips\long.mp4");
        assert!(
            video_url(Some(&server), "id", path, LARGE_VIDEO_BYTES)
                .starts_with("http://127.0.0.1:")
        );
        assert_eq!(
            video_url(Some(&server), "id", path, LARGE_VIDEO_BYTES - 1),
            asset_url(path)
        );
        assert_eq!(
            video_url(None, "id", path, LARGE_VIDEO_BYTES),
            asset_url(path)
        );
    }
}
