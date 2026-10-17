//! URLs the preview player loads videos from (architecture plan 9.2).
//!
//! Every video plays from the loopback media server (see
//! [`crate::media_server`]): it answers ranges directly from the file,
//! with shorter seek tails, 6 to 8 times the throughput of the asset
//! protocol and no slowdown while the window's thread is busy (the
//! measurements are in its documentation). Only when the server
//! could not start do videos play through Tauri's asset protocol, whose
//! scope holds exactly the files the user chose (see [`crate::paths`]).

use std::fmt::Write as _;
use std::path::Path;

use crate::media_server::MediaServer;

/// The URL of the video `path`, served under `id`: the loopback server's
/// when it runs, otherwise the asset protocol's (logged, as it is the
/// slower path). The asset protocol only serves files in its scope.
pub(crate) fn video_url(server: Option<&MediaServer>, id: &str, path: &Path) -> String {
    if let Some(server) = server {
        return server.url(id, path);
    }
    let name = path.file_name().unwrap_or(path.as_os_str());
    tauri_plugin_log::log::info!(
        "{} plays through the asset protocol: the media server is not running",
        name.to_string_lossy()
    );
    asset_url(path)
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
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::path::Path;

    use super::{asset_url, encode_component, video_url};
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

    /// There is no size limit: a tiny video plays from the server too.
    #[test]
    fn videos_of_every_size_play_from_the_loopback_server() {
        let dir = std::env::temp_dir().join(format!("blurrd-media-url-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("tiny.mp4");
        std::fs::write(&path, b"tiny video").unwrap();
        let server = MediaServer::start("http://tauri.localhost").unwrap();

        let url = video_url(Some(&server), "tiny", &path);
        let rest = url.strip_prefix("http://").unwrap();
        let (address, resource) = rest.split_once('/').unwrap();
        assert!(address.starts_with("127.0.0.1:"), "{url}");
        let mut stream = TcpStream::connect(address).unwrap();
        write!(
            stream,
            "GET /{resource} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"
        )
        .unwrap();
        let mut answer = String::new();
        stream.read_to_string(&mut answer).unwrap();
        assert!(answer.starts_with("HTTP/1.1 200 "), "{answer}");
        assert!(answer.ends_with("\r\n\r\ntiny video"), "{answer}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn videos_play_through_the_asset_protocol_without_the_server() {
        let path = Path::new(r"C:\clips\long.mp4");
        assert_eq!(video_url(None, "id", path), asset_url(path));
    }
}
