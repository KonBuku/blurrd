//! The loopback media server (architecture plan 8 and 9.2): the
//! preview's video source, for videos of every size. The asset protocol
//! is used only when this server cannot start: on WebView2 it hands
//! every range through the webview's resource callback in pieces of
//! about 1 MB. Measured in WebView2 155 with GPU decoding (1080p60 H.264,
//! 1.26 GiB): its median far seek was a little faster (22-23 ms against
//! 31 ms), but its tails were slower (90th percentile 103-107 ms against
//! 59 ms, slowest +1.5 s seek 196-208 ms against 63 ms), its first bytes
//! took twice as long (7 ms against 3 ms), it read 6 to 8 times slower
//! (141-147 MB/s against 0.86-1.26 GB/s), held 3 times the host memory,
//! and its seeks slowed to about 440 ms while the window's thread was
//! busy, when this server's did not change.
//!
//! The server listens on 127.0.0.1 only, on a port the system picks,
//! allows only the window's origin to read its answers (CORS), and
//! answers `GET` and `HEAD` for `/<token>/<id>`: the token is random per
//! app start and compared in constant time, the id names a file the
//! backend registered (an opened video or its preview proxy, until the
//! project closes), so no other file can be read.
//! A single range (`bytes=a-b`, `a-`, `-n`) answers 206 with the part, a
//! range past the end 416; other `Range` headers (several ranges, other
//! units, malformed) are ignored and the whole file is sent, as RFC 9110
//! allows. Connections are kept alive, each on its own thread, at most
//! [`MAX_CONNECTIONS`] at a time (more are answered 503 after their
//! request head was read); a request body is read and dropped, or
//! the connection closes when it is large or chunked. A request (head
//! and body) must arrive within [`REQUEST_TIMEOUT`] of its first byte,
//! so clients that trickle it cannot hold the connection slots.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::fs::File;
use std::io::{self, BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::net::{Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::projects::lock;

/// Most connections served at once; the player needs one or two.
pub(crate) const MAX_CONNECTIONS: usize = 16;
/// Connections over the limit answered 503 at once; more are dropped.
const MAX_TURNED_AWAY: usize = 4;
/// How long a connection over the limit may take to send its request,
/// and then to close after the 503.
const TURN_AWAY_TIME: Duration = Duration::from_secs(1);
/// How long an idle kept-alive connection stays open.
const IDLE_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a request may take to arrive, from its first byte.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long writing an answer may stall before the connection closes.
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// Longest request head accepted.
const MAX_HEAD_BYTES: usize = 16 * 1024;
/// Largest request body read and dropped on a kept-alive connection.
const MAX_BODY_BYTES: u64 = 64 * 1024;
/// Longest pause after failed accepts (it doubles from 10 ms).
const MAX_ACCEPT_BACKOFF: Duration = Duration::from_secs(1);
/// Bytes of a file sent per socket write. With `io::copy`'s 8 KiB the
/// webview read about 230 MB/s; 256 KiB writes reach 0.8-1.2 GB/s
/// there and over 2 GB/s for a plain TCP client on this address.
const SEND_CHUNK: usize = 256 * 1024;

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
        Self::start_with(origin, REQUEST_TIMEOUT)
    }

    /// [`MediaServer::start`] with requests that must arrive within
    /// `request_timeout`.
    fn start_with(origin: &str, request_timeout: Duration) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let token = Uuid::new_v4().simple().to_string();
        let files: Files = Arc::default();
        let server = Server {
            origin: Arc::from(origin),
            token: token.clone(),
            files: Arc::clone(&files),
            open: Arc::new(AtomicUsize::new(0)),
            turned_away: Arc::new(AtomicUsize::new(0)),
            request_timeout,
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

    /// Stops serving the file registered under `id`.
    pub(crate) fn forget(&self, id: &str) {
        lock(&self.files).remove(id);
    }
}

/// The accepting side of the server.
#[derive(Clone)]
struct Server {
    origin: Arc<str>,
    token: String,
    files: Files,
    open: Arc<AtomicUsize>,
    /// Connections over the limit being answered 503.
    turned_away: Arc<AtomicUsize>,
    /// How long a request may take to arrive, from its first byte.
    request_timeout: Duration,
}

impl Server {
    fn accept(&self, listener: &TcpListener) {
        let mut backoff = Duration::ZERO;
        for stream in listener.incoming() {
            let stream = match stream {
                Ok(stream) => {
                    backoff = Duration::ZERO;
                    stream
                }
                Err(error) => {
                    // Out of sockets or handles: wait instead of spinning.
                    backoff = (backoff * 2).clamp(Duration::from_millis(10), MAX_ACCEPT_BACKOFF);
                    tauri_plugin_log::log::debug!("media server accept failed: {error}");
                    std::thread::sleep(backoff);
                    continue;
                }
            };
            if self.open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                self.open.fetch_sub(1, Ordering::SeqCst);
                self.turn_away(stream);
                continue;
            }
            let server = self.clone();
            let spawned = std::thread::Builder::new()
                .name("blurrd-media-connection".to_owned())
                .spawn(move || {
                    if let Err(error) = server.serve(&stream) {
                        tauri_plugin_log::log::debug!("media connection ended: {error}");
                    }
                    server.open.fetch_sub(1, Ordering::SeqCst);
                });
            if spawned.is_err() {
                self.open.fetch_sub(1, Ordering::SeqCst);
            }
        }
    }

    /// Answers a connection over the limit with 503 on its own thread;
    /// beyond [`MAX_TURNED_AWAY`] of them it is closed unanswered.
    fn turn_away(&self, stream: TcpStream) {
        if self.turned_away.fetch_add(1, Ordering::SeqCst) >= MAX_TURNED_AWAY {
            self.turned_away.fetch_sub(1, Ordering::SeqCst);
            return;
        }
        let turned_away = Arc::clone(&self.turned_away);
        let spawned = std::thread::Builder::new()
            .name("blurrd-media-busy".to_owned())
            .spawn(move || {
                if let Err(error) = answer_busy(&stream) {
                    tauri_plugin_log::log::debug!("media busy answer failed: {error}");
                }
                turned_away.fetch_sub(1, Ordering::SeqCst);
            });
        if spawned.is_err() {
            self.turned_away.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Answers the requests of one connection until it closes.
    fn serve(&self, stream: &TcpStream) -> io::Result<()> {
        stream.set_write_timeout(Some(WRITE_TIMEOUT))?;
        let mut reader = BufReader::new(stream);
        let mut writer = stream;
        loop {
            // Wait for the next request as long as the connection may
            // idle; from its first byte on, the whole request must
            // arrive within the request timeout.
            reader.get_ref().set_read_timeout(Some(IDLE_TIMEOUT))?;
            if reader.fill_buf()?.is_empty() {
                return Ok(());
            }
            let deadline = Instant::now() + self.request_timeout;
            let Some(request) = read_head(&mut reader, deadline)? else {
                return Ok(());
            };
            // A body the server cannot skip safely ends the connection
            // after the answer, so it is never read as the next request.
            let keep_alive = request.keep_alive() && skip_body(&request, &mut reader, deadline)?;
            let reply = Reply {
                head: request.method == "HEAD",
                keep_alive,
            };
            self.answer(&request, &mut writer, reply)?;
            if !keep_alive {
                return Ok(());
            }
        }
    }

    fn answer(&self, request: &Head, out: &mut impl Write, reply: Reply) -> io::Result<()> {
        if !matches!(request.method.as_str(), "GET" | "HEAD") {
            return respond_status(out, reply, 405, "method not allowed");
        }
        let Some((token, id)) = request.target.trim_start_matches('/').split_once('/') else {
            return respond_status(out, reply, 404, "not found");
        };
        if !same_token(token.as_bytes(), self.token.as_bytes()) {
            return respond_status(out, reply, 403, "forbidden");
        }
        let Some(path) = lock(&self.files).get(id).cloned() else {
            return respond_status(out, reply, 404, "not found");
        };
        let Ok(mut file) = File::open(&path) else {
            return respond_status(out, reply, 404, "not found");
        };
        let size = file.metadata()?.len();
        let range = request
            .header("range")
            .map_or(Range::Whole, |value| parse_range(value, size));
        let (status, start, end) = match range {
            Range::Whole => (200, 0, size.saturating_sub(1)),
            Range::Part(start, end) => (206, start, end),
            Range::Unsatisfiable => {
                let headers = format!("Content-Range: bytes */{size}\r\n");
                return respond(out, reply, 416, "range not satisfiable", &headers, b"");
            }
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
            reply.connection()
        )?;
        if !reply.head && length > 0 {
            file.seek(SeekFrom::Start(start))?;
            send_part(&mut file, length, out)?;
        }
        out.flush()
    }
}

/// Copies the next `length` bytes of `file` to `out` in writes of
/// [`SEND_CHUNK`] bytes (the last one shorter).
///
/// # Errors
///
/// The errors of reading and writing; [`io::ErrorKind::UnexpectedEof`]
/// when the file ends early (it shrank after its length was sent).
fn send_part(file: &mut impl Read, length: u64, out: &mut impl Write) -> io::Result<()> {
    let mut buffer = vec![0; usize::try_from(length).map_or(SEND_CHUNK, |n| n.min(SEND_CHUNK))];
    let mut left = length;
    while left > 0 {
        let chunk = usize::try_from(left).map_or(buffer.len(), |n| n.min(buffer.len()));
        file.read_exact(&mut buffer[..chunk])?;
        out.write_all(&buffer[..chunk])?;
        left -= chunk as u64;
    }
    Ok(())
}

/// Answers 503 on a connection over the limit. The request head is read
/// first and whatever else the client sends is read until it closes:
/// closing a socket with unread data resets the connection, and the
/// client would see the reset instead of the answer.
fn answer_busy(stream: &TcpStream) -> io::Result<()> {
    stream.set_write_timeout(Some(TURN_AWAY_TIME))?;
    let mut reader = BufReader::new(stream);
    // A client that sends nothing in time still gets the answer.
    let head = read_head(&mut reader, Instant::now() + TURN_AWAY_TIME)
        .ok()
        .flatten();
    let reply = Reply {
        head: head.is_some_and(|head| head.method == "HEAD"),
        keep_alive: false,
    };
    let mut out = stream;
    respond(
        &mut out,
        reply,
        503,
        "Service Unavailable",
        "Retry-After: 1\r\n",
        b"busy",
    )?;
    stream.shutdown(Shutdown::Write)?;
    let deadline = Instant::now() + TURN_AWAY_TIME;
    loop {
        arm(&reader, deadline)?;
        let unread = reader.fill_buf()?.len();
        if unread == 0 {
            return Ok(());
        }
        reader.consume(unread);
    }
}

/// How the answer to a request is written.
#[derive(Debug, Clone, Copy)]
struct Reply {
    /// A `HEAD` request: headers only, never a body.
    head: bool,
    /// Whether the connection stays open afterwards.
    keep_alive: bool,
}

impl Reply {
    fn connection(self) -> &'static str {
        if self.keep_alive {
            "keep-alive"
        } else {
            "close"
        }
    }
}

/// A socket whose reads can time out.
trait TimedRead: Read {
    fn set_timeout(&self, timeout: Duration) -> io::Result<()>;
}

impl TimedRead for &TcpStream {
    fn set_timeout(&self, timeout: Duration) -> io::Result<()> {
        self.set_read_timeout(Some(timeout))
    }
}

/// Lets the next read of `reader` wait until `deadline` at most.
///
/// # Errors
///
/// [`io::ErrorKind::TimedOut`] when the deadline has passed.
fn arm<R: TimedRead>(reader: &BufReader<R>, deadline: Instant) -> io::Result<()> {
    let left = deadline.saturating_duration_since(Instant::now());
    if left.is_zero() {
        return Err(io::Error::new(
            io::ErrorKind::TimedOut,
            "the request took too long",
        ));
    }
    reader.get_ref().set_timeout(left)
}

/// Reads one line (with its line feed) into `line` before `deadline`,
/// never more than `limit` bytes; answers the bytes read, 0 at the end
/// of the stream.
fn read_line<R: TimedRead>(
    reader: &mut BufReader<R>,
    line: &mut Vec<u8>,
    limit: usize,
    deadline: Instant,
) -> io::Result<usize> {
    let mut read = 0;
    loop {
        arm(reader, deadline)?;
        let buffer = reader.fill_buf()?;
        if buffer.is_empty() {
            return Ok(read);
        }
        let (taken, done) = match buffer.iter().position(|&byte| byte == b'\n') {
            Some(end) => (end + 1, true),
            None => (buffer.len(), false),
        };
        line.extend_from_slice(&buffer[..taken]);
        reader.consume(taken);
        read += taken;
        if read > limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "request head too long",
            ));
        }
        if done {
            return Ok(read);
        }
    }
}

/// Reads and drops the body of `request` before `deadline`; answers
/// whether the connection can serve another request (no body, or a
/// small one with a length).
fn skip_body<R: TimedRead>(
    request: &Head,
    reader: &mut BufReader<R>,
    deadline: Instant,
) -> io::Result<bool> {
    if request.header("transfer-encoding").is_some() {
        return Ok(false);
    }
    let Some(length) = request.header("content-length") else {
        return Ok(true);
    };
    match length.parse::<u64>() {
        Ok(0) => Ok(true),
        Ok(length) if length <= MAX_BODY_BYTES => {
            let mut left = length;
            while left > 0 {
                arm(reader, deadline)?;
                let available = reader.fill_buf()?.len();
                if available == 0 {
                    return Ok(false);
                }
                let skipped = available.min(usize::try_from(left).unwrap_or(usize::MAX));
                reader.consume(skipped);
                left -= skipped as u64;
            }
            Ok(true)
        }
        _ => Ok(false),
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

/// Reads one request head before `deadline`; `None` when the peer
/// closed the connection.
fn read_head<R: TimedRead>(
    reader: &mut BufReader<R>,
    deadline: Instant,
) -> io::Result<Option<Head>> {
    let mut lines = Vec::new();
    let mut total = 0;
    loop {
        let mut bytes = Vec::new();
        let read = read_line(reader, &mut bytes, MAX_HEAD_BYTES - total, deadline)?;
        if read == 0 {
            return if lines.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        total += read;
        let line = String::from_utf8(bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "request head not UTF-8"))?;
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

/// What a `Range` header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Range {
    /// The whole file: no range the server serves (several ranges,
    /// another unit, a malformed header).
    Whole,
    /// The first and last byte of a single range.
    Part(u64, u64),
    /// A valid single range that lies past the end of the file.
    Unsatisfiable,
}

/// The answer to `Range: value` for a file of `size` bytes (RFC 9110
/// 14.2: a server may ignore any `Range` header, so everything but one
/// valid byte range is ignored).
fn parse_range(value: &str, size: u64) -> Range {
    let Some((unit, spec)) = value.trim().split_once('=') else {
        return Range::Whole;
    };
    if !unit.trim().eq_ignore_ascii_case("bytes") || spec.contains(',') {
        return Range::Whole;
    }
    let Some((first, last)) = spec.split_once('-') else {
        return Range::Whole;
    };
    let digits = |text: &str| -> Option<Option<u64>> {
        let text = text.trim();
        if text.is_empty() {
            Some(None)
        } else if text.bytes().all(|b| b.is_ascii_digit()) {
            text.parse().ok().map(Some)
        } else {
            None
        }
    };
    let (Some(first), Some(last)) = (digits(first), digits(last)) else {
        return Range::Whole;
    };
    match (first, last) {
        (None, None) => Range::Whole,
        // A suffix: the last `n` bytes.
        (None, Some(0)) => Range::Unsatisfiable,
        (None, Some(_)) if size == 0 => Range::Unsatisfiable,
        (None, Some(suffix)) => Range::Part(size.saturating_sub(suffix), size - 1),
        (Some(start), Some(end)) if end < start => Range::Whole,
        (Some(start), _) if start >= size => Range::Unsatisfiable,
        (Some(start), end) => Range::Part(start, end.map_or(size - 1, |end| end.min(size - 1))),
    }
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

fn respond_status(out: &mut impl Write, reply: Reply, status: u16, text: &str) -> io::Result<()> {
    respond(out, reply, status, text, "", text.as_bytes())
}

/// Writes a plain-text answer; the body is left out for `HEAD`, whose
/// headers still state its length.
fn respond(
    out: &mut impl Write,
    reply: Reply,
    status: u16,
    reason: &str,
    headers: &str,
    body: &[u8],
) -> io::Result<()> {
    write!(
        out,
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/plain; charset=utf-8\r\n\
         Content-Length: {}\r\n{headers}Connection: {}\r\n\r\n",
        body.len(),
        reply.connection()
    )?;
    if !reply.head {
        out.write_all(body)?;
    }
    out.flush()
}

#[cfg(test)]
mod tests {
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream, UdpSocket};
    use std::path::PathBuf;
    use std::time::Duration;

    use super::{
        MAX_CONNECTIONS, MediaServer, Range, SEND_CHUNK, parse_range, same_token, send_part,
    };

    /// The window's origin in the tests.
    pub(crate) const ORIGIN: &str = "http://tauri.localhost";

    #[test]
    fn ranges_parse_like_rfc_9110() {
        let part = Range::Part;
        assert_eq!(parse_range("bytes=0-99", 1000), part(0, 99));
        assert_eq!(parse_range("bytes=900-", 1000), part(900, 999));
        assert_eq!(parse_range("bytes=-100", 1000), part(900, 999));
        assert_eq!(parse_range("bytes=-5000", 1000), part(0, 999));
        assert_eq!(parse_range("bytes=990-2000", 1000), part(990, 999));
        assert_eq!(parse_range("Bytes = 1-2", 1000), part(1, 2));
        // Valid, but past the end.
        assert_eq!(parse_range("bytes=1000-", 1000), Range::Unsatisfiable);
        assert_eq!(parse_range("bytes=-0", 1000), Range::Unsatisfiable);
        assert_eq!(parse_range("bytes=0-", 0), Range::Unsatisfiable);
        // Ignored: the whole file is sent.
        for ignored in [
            "bytes=5-1",
            "bytes=0-1,5-6",
            "items=0-1",
            "bytes=",
            "bytes=-",
            "bytes=x-5",
            "bytes=+1-5",
            "bytes=1-5x",
            "bytes 1-5",
            "bytes=99999999999999999999999-",
        ] {
            assert_eq!(parse_range(ignored, 1000), Range::Whole, "{ignored}");
        }
    }

    /// The sizes of the writes made to it.
    #[derive(Default)]
    struct Writes {
        sizes: Vec<usize>,
        bytes: Vec<u8>,
    }

    impl Write for Writes {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.sizes.push(buf.len());
            self.bytes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// Throughput guard: a file part leaves in [`SEND_CHUNK`] writes, not
    /// in `io::copy`'s 8 KiB ones, which held the webview to ~230 MB/s.
    #[test]
    fn parts_are_sent_in_large_writes() {
        assert_eq!(SEND_CHUNK, 256 * 1024);
        let bytes: Vec<u8> = (0..SEND_CHUNK * 5 / 2 + 7)
            .map(|i| (i % 251) as u8)
            .collect();
        let mut file = std::io::Cursor::new(&bytes);
        file.set_position(5);
        let length = (bytes.len() - 10) as u64;
        let mut out = Writes::default();
        send_part(&mut file, length, &mut out).unwrap();
        assert_eq!(out.bytes, &bytes[5..bytes.len() - 5]);
        assert_eq!(
            out.sizes,
            [SEND_CHUNK, SEND_CHUNK, bytes.len() - 10 - 2 * SEND_CHUNK]
        );
        // A small part is one write of its own size.
        let mut out = Writes::default();
        send_part(&mut std::io::Cursor::new(&bytes), 10, &mut out).unwrap();
        assert_eq!(out.sizes, [10]);
        // A file that ends early fails instead of sending short.
        let mut out = Writes::default();
        let short = send_part(&mut std::io::Cursor::new(&bytes[..100]), 200, &mut out);
        assert_eq!(short.unwrap_err().kind(), std::io::ErrorKind::UnexpectedEof);
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

    fn video(name: &str) -> (PathBuf, Vec<u8>) {
        let dir = std::env::temp_dir().join(format!("blurrd-media-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("{name}.mp4"));
        let bytes: Vec<u8> = (0..100_000u32).map(|i| (i % 251) as u8).collect();
        std::fs::write(&path, &bytes).unwrap();
        (path, bytes)
    }

    #[test]
    fn serves_registered_files_with_ranges_to_token_holders_only() {
        let (path, bytes) = video("ranges");
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

    /// HEAD answers, request bodies and ignored ranges keep a kept-alive
    /// connection in step: every answer still parses.
    #[test]
    fn connections_stay_in_step() {
        let (path, bytes) = video("in-step");
        let server = MediaServer::start(ORIGIN).unwrap();
        let url = server.url("abc", &path);
        let resource = format!("/{}", url.splitn(4, '/').nth(3).unwrap());
        let token = resource.split('/').nth(1).unwrap().to_owned();
        let mut stream = TcpStream::connect(server.address).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let get =
            |range: &str| format!("GET {resource} HTTP/1.1\r\nHost: 127.0.0.1\r\n{range}\r\n");

        // Ranges the server does not serve: the whole file (RFC 9110).
        for ignored in ["bytes=0-1,5-6", "items=0-9", "bytes=9-1", "bytes=a-"] {
            let answer = ask(
                &mut stream,
                &mut reader,
                &get(&format!("Range: {ignored}\r\n")),
            );
            assert_eq!(answer.status, 200, "{ignored}");
            assert_eq!(answer.body, bytes, "{ignored}");
        }
        // HEAD answers carry no body, errors included: the next answer on
        // the connection still parses.
        for target in [resource.clone(), format!("/{token}/other"), "/x".to_owned()] {
            let head = ask(
                &mut stream,
                &mut reader,
                &format!("HEAD {target} HTTP/1.1\r\nHost: x\r\n\r\n"),
            );
            assert!(matches!(head.status, 200 | 404), "{target}");
        }
        // A small request body is skipped, so it is not read as a request.
        let body = "GET /x HTTP/1.1\r\n\r\n";
        let posted = ask(
            &mut stream,
            &mut reader,
            &format!(
                "POST {resource} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
                body.len()
            ),
        );
        assert_eq!(posted.status, 405);
        let after = ask(&mut stream, &mut reader, &get("Range: bytes=0-9\r\n"));
        assert_eq!(after.status, 206);
        assert_eq!(after.body, &bytes[..10]);

        // A chunked body closes the connection after the answer.
        let chunked = ask(
            &mut stream,
            &mut reader,
            &format!(
                "GET {resource} HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\
                 Range: bytes=0-0\r\n\r\n5\r\nhello\r\n0\r\n\r\n"
            ),
        );
        assert_eq!(chunked.status, 206);
        assert_eq!(chunked.header("connection"), Some("close"));
        let mut rest = Vec::new();
        assert_eq!(reader.read_to_end(&mut rest).unwrap(), 0);

        // The file of a closed project is no longer served.
        server.forget("abc");
        let mut stream = TcpStream::connect(server.address).unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        assert_eq!(ask(&mut stream, &mut reader, &get("")).status, 404);
        let _ = std::fs::remove_file(&path);
    }

    /// Clients that trickle a request head lose their connection once
    /// the request timeout passes, so they cannot hold every slot; a
    /// slow request that arrives in time is answered.
    #[test]
    fn trickled_requests_cannot_hold_the_slots() {
        let (path, bytes) = video("trickle");
        let server = MediaServer::start_with(ORIGIN, Duration::from_millis(400)).unwrap();
        let url = server.url("abc", &path);
        let resource = format!("/{}", url.splitn(4, '/').nth(3).unwrap());
        let get = format!("GET {resource} HTTP/1.1\r\nHost: x\r\nRange: bytes=0-9\r\n\r\n");

        // Every slot taken by a client that sends one line after another
        // and never ends its head. The lines do not start like an HTTP
        // request: some security products on Windows hold back a partial
        // HTTP head, also on the loopback address, until it is complete.
        let mut slow: Vec<TcpStream> = (0..MAX_CONNECTIONS)
            .map(|_| {
                let mut stream = TcpStream::connect(server.address).unwrap();
                stream.write_all(b"SLOW /x\r\n").unwrap();
                stream
            })
            .collect();
        for _ in 0..12 {
            std::thread::sleep(Duration::from_millis(100));
            for stream in &mut slow {
                // Fails once the server has closed the connection.
                let _ = stream.write_all(b"X-Slow: 1\r\n");
            }
        }
        for stream in &mut slow {
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut rest = Vec::new();
            if let Err(error) = stream.read_to_end(&mut rest) {
                assert!(
                    !matches!(
                        error.kind(),
                        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                    ),
                    "a trickling client kept its connection"
                );
            }
        }
        drop(slow);

        // The slots are free again; a head sent in parts within the
        // timeout is answered.
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let mut stream = TcpStream::connect(server.address).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let (first, second) = get.split_at(get.len() / 2);
            stream.write_all(first.as_bytes()).unwrap();
            std::thread::sleep(Duration::from_millis(100));
            stream.write_all(second.as_bytes()).unwrap();
            let answer = read_answer(&mut reader, false);
            if answer.status == 206 {
                assert_eq!(answer.body, &bytes[..10]);
                break;
            }
            assert!(std::time::Instant::now() < deadline, "{}", answer.status);
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = std::fs::remove_file(&path);
    }

    /// A client over the connection limit reads a whole 503 answer,
    /// not a reset, also when it sent more than its request head.
    #[test]
    fn connections_over_the_limit_are_answered_503() {
        let (path, _) = video("busy");
        let server = MediaServer::start(ORIGIN).unwrap();
        let url = server.url("abc", &path);
        let resource = format!("/{}", url.splitn(4, '/').nth(3).unwrap());
        // Idle clients take every slot (they may idle for 30 s).
        let idle: Vec<TcpStream> = (0..MAX_CONNECTIONS)
            .map(|_| TcpStream::connect(server.address).unwrap())
            .collect();

        for method in ["GET", "HEAD"] {
            let mut stream = TcpStream::connect(server.address).unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(10)))
                .unwrap();
            // A request with a body, then a second request: data the
            // server never answers.
            let request = format!(
                "{method} {resource} HTTP/1.1\r\nHost: x\r\nContent-Length: 4\r\n\r\nbody\
                 GET {resource} HTTP/1.1\r\nHost: x\r\n\r\n"
            );
            stream.write_all(request.as_bytes()).unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let answer = read_answer(&mut reader, method == "HEAD");
            assert_eq!(answer.status, 503, "{method}");
            assert_eq!(answer.header("connection"), Some("close"), "{method}");
            assert_eq!(answer.header("retry-after"), Some("1"), "{method}");
            if method == "GET" {
                assert_eq!(answer.body, b"busy");
            }
            // Then the server closes cleanly: no body after a HEAD
            // answer and no reset.
            let mut rest = Vec::new();
            reader.read_to_end(&mut rest).unwrap();
            assert!(rest.is_empty(), "{method}: {rest:?}");
        }
        drop(idle);
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
