//! The loopback media server (architecture plan 8 and 9.2): the preview
//! player's fallback for large videos, which the asset protocol serves
//! poorly on WebView2 (every range request goes through the webview's
//! resource callback and its body is held in memory).
//!
//! The server listens on 127.0.0.1 only, on a port the system picks,
//! allows only the window's origin to read its answers (CORS), and
//! answers `GET` and `HEAD` for `/<token>/<id>`: the token is random per
//! app start and compared in constant time, the id names a file the
//! backend registered (an opened video), so no other file can be read.
//! Range requests (`bytes=a-b`, `a-`, `-n`) answer 206 with the part,
//! ranges past the end 416. Connections are kept alive, each on its own
//! thread, at most [`MAX_CONNECTIONS`] at a time.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use uuid::Uuid;

use crate::projects::lock;

/// Most connections served at once; the player needs one or two.
pub(crate) const MAX_CONNECTIONS: usize = 16;
/// How long an idle kept-alive connection stays open.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest request head accepted.
const MAX_HEAD_BYTES: usize = 16 * 1024;

/// The registered files by id.
type Files = Arc<Mutex<HashMap<String, PathBuf>>>;

/// A running media server.
#[derive(Debug)]
pub(crate) struct MediaServer {
    address: SocketAddr,
    token: String,
    files: Files,
}

impl MediaServer {
    /// Binds 127.0.0.1 on a free port and serves the page of the window
    /// at `origin` on a background thread.
    ///
    /// # Errors
    ///
    /// The errors of binding the socket or starting the thread.
    pub(crate) fn start(origin: &str) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let token = Uuid::new_v4().simple().to_string();
        let files: Files = Arc::default();
        let server = Server {
            origin: Arc::from(origin),
            token: token.clone(),
            files: Arc::clone(&files),
            open: Arc::new(AtomicUsize::new(0)),
        };
        std::thread::Builder::new()
            .name("blurrd-media".to_owned())
            .spawn(move || server.accept(&listener))?;
        Ok(Self {
            address,
            token,
            files,
        })
    }

    /// Serves `path` under `id` and answers its URL.
    pub(crate) fn url(&self, id: &str, path: &Path) -> String {
        lock(&self.files).insert(id.to_owned(), path.to_path_buf());
        format!("http://{}/{}/{id}", self.address, self.token)
    }
}

/// The accepting side of the server.
#[derive(Clone)]
struct Server {
    origin: Arc<str>,
    token: String,
    files: Files,
    open: Arc<AtomicUsize>,
}

impl Server {
    fn accept(&self, listener: &TcpListener) {
        for stream in listener.incoming() {
            let Ok(stream) = stream else { continue };
            if self.open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                self.open.fetch_sub(1, Ordering::SeqCst);
                let _ = respond_status(&mut &stream, 503, "busy", false);
                continue;
            }
            let server = self.clone();
            let spawned = std::thread::Builder::new()
                .name("blurrd-media-connection".to_owned())
                .spawn(move || {
                    if let Err(error) = server.serve(stream) {
                        tauri_plugin_log::log::debug!("media connection ended: {error}");
                    }
                    server.open.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                self.open.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Answers the requests of one connection until it closes.
    fn serve(&self, stream: TcpStream) -> io::Result<()> {
        stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
        let mut reader = BufReader::new(stream.try_clone()?);
        let mut writer = stream;
        loop {
            let Some(request) = read_head(&mut reader)? else {
                return Ok(());
            };
            let keep_alive = request.keep_alive();
            self.answer(&request, &mut writer, keep_alive)?;
            if !keep_alive {
                return Ok(());
            }
        }
    }

    fn answer(&self, request: &Head, out: &mut impl Write, keep_alive: bool) -> io::Result<()> {
        let head = match request.method.as_str() {
            "GET" => false,
            "HEAD" => true,
            _ => return respond_status(out, 405, "method not allowed", keep_alive),
        };
        let Some((token, id)) = request.target.trim_start_matches('/').split_once('/') else {
            return respond_status(out, 404, "not found", keep_alive);
        };
        if !same_token(token.as_bytes(), self.token.as_bytes()) {
            return respond_status(out, 403, "forbidden", keep_alive);
        }
        let Some(path) = lock(&self.files).get(id).cloned() else {
            return respond_status(out, 404, "not found", keep_alive);
        };
        let Ok(mut file) = File::open(&path) else {
            return respond_status(out, 404, "not found", keep_alive);
        };
        let size = file.metadata()?.len();
        let range = match request
            .header("range")
            .map(|value| parse_range(value, size))
        {
            None => None,
            Some(Some(range)) => Some(range),
            Some(None) => {
                let headers = format!("Content-Range: bytes */{size}\r\n");
                return respond(out, 416, "range not satisfiable", &headers, b"", keep_alive);
            }
        };
        let (status, start, end) = match range {
            Some((start, end)) => (206, start, end),
            None => (200, 0, size.saturating_sub(1)),
        };
        let length = if size == 0 { 0 } else { end - start + 1 };
        let mut headers = format!(
            "Content-Type: {}\r\nContent-Length: {length}\r\nAccept-Ranges: bytes\r\n\
             Access-Control-Allow-Origin: {}\r\nCache-Control: no-cache\r\n",
            content_type(&path),
            self.origin
        );
        if status == 206 {
            let _ = write!(headers, "Content-Range: bytes {start}-{end}/{size}\r\n");
        }
        write!(
            out,
            "HTTP/1.1 {status} {}\r\n{headers}Connection: {}\r\n\r\n",
            if status == 206 {
                "Partial Content"
            } else {
                "OK"
            },
            if keep_alive { "keep-alive" } else { "close" }
        )?;
        if !head && length > 0 {
            file.seek(SeekFrom::Start(start))?;
            io::copy(&mut (&mut file).take(length), out)?;
        }
        out.flush()
    }
}

/// A parsed request head.
#[derive(Debug)]
struct Head {
    method: String,
    target: String,
    version: String,
    headers: Vec<(String, String)>,
}

impl Head {
    /// The value of header `name` (case-insensitive).
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// Whether the connection stays open after the answer.
    fn keep_alive(&self) -> bool {
        let connection = self.header("connection").map(str::to_ascii_lowercase);
        match connection.as_deref() {
            Some("close") => false,
            Some("keep-alive") => true,
            _ => self.version == "HTTP/1.1",
        }
    }
}

/// Reads one request head; `None` when the peer closed the connection.
fn read_head(reader: &mut impl BufRead) -> io::Result<Option<Head>> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut line = String::new();
        let read = reader.read_line(&mut line)?;
        if read == 0 {
            return if lines.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        total += read;
        if total > MAX_HEAD_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request head too long",
            ));
        }
        let line = line.trim_end_matches(['\r', '\n']).to_owned();
        if line.is_empty() {
            if lines.is_empty() {
                continue;
            }
            break;
        }
        lines.push(line);
    }
    let mut request_line = lines[0].split_whitespace();
    let (Some(method), Some(target), Some(version)) = (
        request_line.next(),
        request_line.next(),
        request_line.next(),
    ) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "bad request line",
        ));
    };
    let headers = lines[1..]
        .iter()
        .filter_map(|line| line.split_once(':'))
        .map(|(key, value)| (key.trim().to_owned(), value.trim().to_owned()))
        .collect();
    Ok(Some(Head {
        method: method.to_owned(),
        target: target.to_owned(),
        version: version.to_owned(),
        headers,
    }))
}

/// The first and last byte of a `Range` header for a file of `size`
/// bytes; `None` when the range cannot be satisfied. Only single ranges
/// are supported (the player asks for nothing else).
fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (first, last) = spec.split_once('-')?;
    let (first, last) = (first.trim(), last.trim());
    if first.is_empty() {
        let suffix: u64 = last.parse().ok()?;
        if suffix == 0 {
            return None;
        }
        return Some((size.saturating_sub(suffix), size - 1));
    }
    let start: u64 = first.parse().ok()?;
    let end = if last.is_empty() {
        size - 1
    } else {
        last.parse::<u64>().ok()?.min(size - 1)
    };
    (start <= end).then_some((start, end))
}

/// Compares two tokens in time independent of where they differ.
fn same_token(given: &[u8], expected: &[u8]) -> bool {
    given.len() == expected.len()
        && given
            .iter()
            .zip(expected)
            .fold(0u8, |diff, (a, b)| diff | (a ^ b))
            == 0
}

/// The media type of a video by its extension.
fn content_type(path: &Path) -> &'static str {
    let extension = path
        .extension()
        .map(|ext| ext.to_string_lossy().to_ascii_lowercase());
    match extension.as_deref() {
        Some("mp4" | "m4v") => "video/mp4",
        Some("mov") => "video/quicktime",
        Some("mkv") => "video/x-matroska",
        _ => "application/octet-stream",
    }
}

fn respond_status(
    out: &mut impl Write,
    status: u16,
    text: &str,
    keep_alive: bool,
) -> io::Result<()> {
    respond(out, status, text, "", text.as_bytes(), keep_alive)
}

fn respond(
    out: &mut impl Write,
    status: u16,
    reason: &str,
    headers: &str,
    body: &[u8],
    keep_alive: bool,
) -> io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n{headers}Connection: {}\r\n\r\n",
        body.len(),
        if keep_alive { "keep-alive" } else { "close" }
    )?;
    out.write_all(body)?;
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream, UdpSocket};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{MediaServer, parse_range, same_token};

    /// The window's origin in the tests.
    pub(crate) const ORIGIN: &str = "http://tauri.localhost";

    #[test]
    fn ranges_parse_like_rfc_9110() {
        assert_eq!(parse_range("bytes=0-99", 1000), Some((0, 99)));
        assert_eq!(parse_range("bytes=900-", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-100", 1000), Some((900, 999)));
        assert_eq!(parse_range("bytes=-5000", 1000), Some((0, 999)));
        assert_eq!(parse_range("bytes=990-2000", 1000), Some((990, 999)));
        assert_eq!(parse_range("bytes=1000-", 1000), None);
        assert_eq!(parse_range("bytes=5-1", 1000), None);
        assert_eq!(parse_range("bytes=0-1,5-6", 1000), None);
        assert_eq!(parse_range("items=0-1", 1000), None);
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(same_token(b"abc", b"abc"));
        assert!(!same_token(b"abx", b"abc"));
        assert!(!same_token(b"ab", b"abc"));
    }

    /// One response: status, headers (lower-case names) and body.
    struct Answer {
        status: u16,
        headers: Vec<(String, String)>,
        body: Vec<u8>,
    }

    impl Answer {
        fn header(&self, name: &str) -> Option<&str> {
            self.headers
                .iter()
                .find(|(key, _)| key == name)
                .map(|(_, value)| value.as_str())
        }
    }

    fn read_answer(reader: &mut BufReader<TcpStream>, head_only: bool) -> Answer {
        let mut status_line = String::new();
        reader.read_line(&mut status_line).unwrap();
        let status = status_line
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse()
            .unwrap();
        let mut headers = Vec::new();
        loop {
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            let (key, value) = line.split_once(':').unwrap();
            headers.push((key.to_ascii_lowercase(), value.trim().to_owned()));
        }
        let mut answer = Answer {
            status,
            headers,
            body: Vec::new(),
        };
        if !head_only {
            let length: usize = answer.header("content-length").unwrap().parse().unwrap();
            answer.body = vec![0; length];
            reader.read_exact(&mut answer.body).unwrap();
        }
        answer
    }

    /// Sends `request` on `stream` and reads the answer.
    fn ask(stream: &mut TcpStream, reader: &mut BufReader<TcpStream>, request: &str) -> Answer {
        stream.write_all(request.as_bytes()).unwrap();
        read_answer(reader, request.starts_with("HEAD"))
    }

    fn video() -> (PathBuf, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("blurrd-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("clip.mp4");
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    #[test]
    fn serves_registered_files_with_ranges_to_token_holders_only() {
        let (path, bytes) = video();
        let server = MediaServer::start(ORIGIN).unwrap();
        let url = server.url("abc", &path);
        assert!(url.starts_with("http://127.0.0.1:"), "{url}");
        let target = url.split_once("127.0.0.1:").unwrap().1;
        let (port, resource) = target.split_once('/').unwrap();
        let resource = format!("/{resource}");
        let token = resource.split('/').nth(1).unwrap().to_owned();

        let mut stream = TcpStream::connect(("127.0.0.1", port.parse::<u16>().unwrap())).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let get =
            |range: &str| format!("GET {resource} HTTP/1.1\r\nHost: 127.0.0.1\r\n{range}\r\n");

        // Whole file, then ranges on the same kept-alive connection.
        let whole = ask(&mut stream, &mut reader, &get(""));
        assert_eq!(whole.status, 200);
        assert_eq!(whole.body, bytes);
        assert_eq!(whole.header("accept-ranges"), Some("bytes"));
        assert_eq!(whole.header("content-type"), Some("video/mp4"));
        assert_eq!(whole.header("access-control-allow-origin"), Some(ORIGIN));
        let middle = ask(&mut stream, &mut reader, &get("Range: bytes=1000-1999\r\n"));
        assert_eq!(middle.status, 206);
        assert_eq!(middle.body, &bytes[1000..2000]);
        assert_eq!(
            middle.header("content-range"),
            Some("bytes 1000-1999/100000")
        );
        let tail = ask(&mut stream, &mut reader, &get("Range: bytes=-10\r\n"));
        assert_eq!(tail.body, &bytes[99_990..]);
        let open = ask(&mut stream, &mut reader, &get("Range: bytes=99000-\r\n"));
        assert_eq!(open.body, &bytes[99_000..]);
        let past = ask(&mut stream, &mut reader, &get("Range: bytes=200000-\r\n"));
        assert_eq!(past.status, 416);
        assert_eq!(past.header("content-range"), Some("bytes */100000"));
        let head = ask(
            &mut stream,
            &mut reader,
            &format!("HEAD {resource} HTTP/1.1\r\nHost: x\r\n\r\n"),
        );
        assert_eq!(head.status, 200);
        assert_eq!(head.header("content-length"), Some("100000"));

        // Without the token, with a wrong one or an unknown id: nothing.
        for (bad, status) in [
            ("/abc".to_owned(), 404),
            (format!("/{}/abc", "0".repeat(token.len())), 403),
            ("/wrong/abc".to_owned(), 403),
            (format!("/{token}/other"), 404),
            (format!("/{token}/../abc"), 404),
        ] {
            let answer = ask(
                &mut stream,
                &mut reader,
                &format!("GET {bad} HTTP/1.1\r\nHost: x\r\n\r\n"),
            );
            assert_eq!(answer.status, status, "{bad}");
        }
        let posted = ask(
            &mut stream,
            &mut reader,
            &format!("POST {resource} HTTP/1.1\r\nContent-Length: 0\r\n\r\n"),
        );
        assert_eq!(posted.status, 405);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn listens_on_the_loopback_address_only() {
        let server = MediaServer::start(ORIGIN).unwrap();
        assert!(server.address.ip().is_loopback());
        // The address of this machine on its network (connecting a UDP
        // socket sends nothing); the server must not answer there.
        let probe = UdpSocket::bind(("0.0.0.0", 0)).unwrap();
        let Ok(()) = probe.connect(("192.0.2.1", 9)) else {
            eprintln!("skipped: no network address");
            return;
        };
        let local = probe.local_addr().unwrap().ip();
        if local.is_unspecified() || local.is_loopback() {
            eprintln!("skipped: no network address");
            return;
        }
        let outside = SocketAddr::new(local, server.address.port());
        assert!(TcpStream::connect_timeout(&outside, Duration::from_millis(500)).is_err());
    }

    /// Seeks in a large video (`BLURRD_LARGE_VIDEO`, 1 GiB or more) as
    /// the player does: random 2 MiB ranges on one kept-alive connection.
    /// Prints the time to the first byte and to the last byte against
    /// reading the same ranges from the file directly.
    #[test]
    #[ignore = "needs BLURRD_LARGE_VIDEO; run in release"]
    fn seeks_in_a_large_video() {
        use std::io::{Seek, SeekFrom};
        use std::time::Instant;

        const RANGE: u64 = 2 << 20;

        let Some(path) = std::env::var_os("BLURRD_LARGE_VIDEO").map(PathBuf::from) else {
            eprintln!("skipped: BLURRD_LARGE_VIDEO is not set");
            return;
        };
        let size = std::fs::metadata(&path).unwrap().len();
        assert!(size >= 1 << 30, "{size}");
        let server = MediaServer::start(ORIGIN).unwrap();
        let url = server.url("large", &path);
        let resource = format!("/{}", url.splitn(4, '/').nth(3).unwrap());
        let mut stream = TcpStream::connect(server.address).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        // A fixed pseudo-random walk over the file (no RNG dependency).
        let mut state = 0x9e37_79b9_7f4a_7c15_u64;
        let mut positions = Vec::new();
        for _ in 0..60 {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            positions.push(state % (size - RANGE));
        }
        let mut first_byte = Vec::new();
        let mut whole = Vec::new();
        for &start in &positions {
            let asked = Instant::now();
            let request = format!(
                "GET {resource} HTTP/1.1\r\nHost: x\r\nRange: bytes={start}-{}\r\n\r\n",
                start + RANGE - 1
            );
            stream.write_all(request.as_bytes()).unwrap();
            let answer = read_answer(&mut reader, false);
            assert_eq!(answer.status, 206);
            whole.push(asked.elapsed());
            assert_eq!(answer.body.len() as u64, RANGE);
        }
        for &start in positions.iter().take(20) {
            let asked = Instant::now();
            let request = format!(
                "GET {resource} HTTP/1.1\r\nHost: x\r\nRange: bytes={start}-{}\r\n\r\n",
                start + RANGE - 1
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut status = String::new();
            reader.read_line(&mut status).unwrap();
            first_byte.push(asked.elapsed());
            let mut length = 0;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line.trim().is_empty() {
                    break;
                }
                if let Some((key, value)) = line.split_once(':')
                    && key.eq_ignore_ascii_case("content-length")
                {
                    length = value.trim().parse().unwrap();
                }
            }
            let mut body = vec![0; length];
            reader.read_exact(&mut body).unwrap();
        }
        let mut direct = Vec::new();
        let mut file = std::fs::File::open(&path).unwrap();
        let mut buffer = vec![0; RANGE as usize];
        for &start in positions.iter().rev() {
            let asked = Instant::now();
            file.seek(SeekFrom::Start(start)).unwrap();
            file.read_exact(&mut buffer).unwrap();
            direct.push(asked.elapsed());
        }
        let ms = |mut times: Vec<Duration>| {
            times.sort_unstable();
            let pick = |q: usize| times[(times.len() - 1) * q / 100].as_secs_f64() * 1e3;
            format!(
                "median {:.2} ms, p95 {:.2} ms, max {:.2} ms",
                pick(50),
                pick(95),
                pick(100)
            )
        };
        eprintln!(
            "large video {:.2} GiB, 2 MiB seeks: first byte {}; whole range {}; file read {}",
            size as f64 / f64::from(1u32 << 30),
            ms(first_byte),
            ms(whole.clone()),
            ms(direct)
        );
        assert!(whole.iter().all(|t| *t < Duration::from_millis(500)));
    }
}
